//! glTF 2.0(GLB)の読み込み。三角形メッシュの位置・法線・頂点色・マテリアルの基本色(`baseColorFactor`)だけを取り出し、
//! シーンの全ノードの変換を頂点に焼き込んで、1つの`ModelMesh`にまとめる。
//!
//! 対応しないもの(読み飛ばす。エラーにはしない): テクスチャ・アニメーション・スキン・モーフ・カメラ・光源、
//! 三角形以外のプリミティブ(線・点)。外部ファイル参照やdata URIのバッファは対応しない(エラー。GLBにする)。
//! 画像のデコードが要らないので、`gltf`クレートは`image`を使う`import`機能なしで依存している。
//!
//! 座標: glTFは+Y上・+Z前(右手系)。機体座標(x=右、y=前、z=上)へ`(x,y,z) → (-x, z, y)`で変換する
//! (回転だけで鏡映ではない。`(右, 前, 上)`は東・北・上と同じ右手系)。

use glam::{Mat3, Mat4, Vec3};

use super::types::{ModelMesh, ModelVertex};

/// 頂点数の上限(これを超えるモデルは読み込まない。重すぎるファイルでブラウザを止めないため)。
const MAX_VERTICES: usize = 500_000;
/// ノードの入れ子の深さの上限(壊れた/悪意のあるファイルで再帰が深くなりすぎないため)。
const MAX_NODE_DEPTH: usize = 128;

/// GLB(バイナリglTF)のバイト列から`ModelMesh`を作る。
pub(crate) fn import_glb(bytes: &[u8]) -> Result<ModelMesh, String> {
    let gltf = gltf::Gltf::from_slice(bytes).map_err(|e| format!("glTFの解析に失敗: {e}"))?;
    let mut buffers: Vec<&[u8]> = Vec::new();
    for buffer in gltf.buffers() {
        match buffer.source() {
            gltf::buffer::Source::Bin => {
                buffers.push(gltf.blob.as_deref().ok_or("GLBにバイナリチャンクがありません")?);
            }
            gltf::buffer::Source::Uri(_) => {
                return Err("外部ファイルやdata URIのバッファは対応していません(.glb形式で書き出してください)".to_string());
            }
        }
    }
    let scene = gltf.default_scene().or_else(|| gltf.scenes().next()).ok_or("glTFにシーンがありません")?;
    let mut builder = Builder::default();
    for node in scene.nodes() {
        builder.visit(&node, Mat4::IDENTITY, &buffers, 0)?;
    }
    builder.finish()
}

#[derive(Default)]
struct Builder {
    vertices: Vec<ModelVertex>,
    indices: Vec<u32>,
}

/// glTFの座標を機体座標へ(右=-x、前=z、上=y)。
fn to_body(v: Vec3) -> [f32; 3] {
    [-v.x, v.z, v.y]
}

impl Builder {
    fn visit(&mut self, node: &gltf::Node, parent: Mat4, buffers: &[&[u8]], depth: usize) -> Result<(), String> {
        if depth > MAX_NODE_DEPTH {
            return Err("ノードの入れ子が深すぎます".to_string());
        }
        let world = parent * Mat4::from_cols_array_2d(&node.transform().matrix());
        if let Some(mesh) = node.mesh() {
            for primitive in mesh.primitives() {
                self.add_primitive(&primitive, world, buffers)?;
            }
        }
        for child in node.children() {
            self.visit(&child, world, buffers, depth + 1)?;
        }
        Ok(())
    }

    fn add_primitive(&mut self, primitive: &gltf::Primitive, world: Mat4, buffers: &[&[u8]]) -> Result<(), String> {
        if primitive.mode() != gltf::mesh::Mode::Triangles {
            return Ok(());
        }
        let reader = primitive.reader(|buffer| buffers.get(buffer.index()).copied());
        let positions: Vec<[f32; 3]> = reader.read_positions().ok_or("POSITIONのないメッシュがあります")?.collect();
        let normals: Option<Vec<[f32; 3]>> = reader.read_normals().map(|n| n.collect());
        let colors: Option<Vec<[f32; 4]>> = reader.read_colors(0).map(|c| c.into_rgba_f32().collect());
        let mut indices: Vec<u32> = match reader.read_indices() {
            Some(i) => i.into_u32().collect(),
            None => (0..positions.len() as u32).collect(),
        };
        indices.truncate(indices.len() / 3 * 3);
        if indices.iter().any(|&i| i as usize >= positions.len()) {
            return Err("頂点の範囲外を指すインデックスがあります".to_string());
        }
        if normals.as_ref().is_some_and(|n| n.len() != positions.len())
            || colors.as_ref().is_some_and(|c| c.len() != positions.len())
        {
            return Err("頂点属性の数が一致しません".to_string());
        }
        let base = primitive.material().pbr_metallic_roughness().base_color_factor();

        // 法線の変換は、拡大縮小・せん断があっても向きを保つ「逆転置行列」。特異な行列(潰れたノード)は、そのまま使う。
        let linear = Mat3::from_mat4(world);
        let normal_matrix = if linear.determinant().abs() > 1e-12 { linear.inverse().transpose() } else { linear };
        let convert = |position: [f32; 3], normal: Vec3, color: [f32; 4]| ModelVertex {
            position: to_body(world.transform_point3(Vec3::from(position))),
            normal: to_body((normal_matrix * normal).normalize_or_zero()),
            color: [color[0] * base[0], color[1] * base[1], color[2] * base[2], 1.0],
        };
        let color_of = |i: usize| colors.as_ref().map_or([1.0; 4], |c| c[i]);

        if self.vertices.len() + indices.len().max(positions.len()) > MAX_VERTICES {
            return Err(format!("頂点数が多すぎます(上限{MAX_VERTICES})"));
        }
        let first = self.vertices.len() as u32;
        match normals {
            // 法線があれば、頂点を共有したまま(なめらかな陰影)。
            Some(normals) => {
                for (i, position) in positions.iter().enumerate() {
                    self.vertices.push(convert(*position, Vec3::from(normals[i]), color_of(i)));
                }
                self.indices.extend(indices.iter().map(|&i| first + i));
            }
            // 無ければ、三角形ごとに頂点を分けて面の法線を付ける(角ばった陰影)。
            None => {
                for triangle in indices.chunks_exact(3) {
                    let p = [triangle[0], triangle[1], triangle[2]].map(|i| Vec3::from(positions[i as usize]));
                    let face = (p[1] - p[0]).cross(p[2] - p[0]);
                    for &i in triangle {
                        self.indices.push(self.vertices.len() as u32);
                        self.vertices.push(convert(positions[i as usize], face, color_of(i as usize)));
                    }
                }
            }
        }
        Ok(())
    }

    fn finish(self) -> Result<ModelMesh, String> {
        if self.indices.is_empty() {
            return Err("描画できる三角形メッシュがありません".to_string());
        }
        let radius_m = self.vertices.iter().map(|v| Vec3::from(v.position).length()).fold(0.0, f32::max);
        if !radius_m.is_finite() || radius_m <= 0.0 {
            return Err("モデルの大きさが0または不正です".to_string());
        }
        Ok(ModelMesh { vertices: self.vertices, indices: self.indices, radius_m })
    }
}

#[cfg(test)]
pub(crate) mod tests {
    use super::*;

    /// テスト用の最小のGLBを作る: 1つの三角形(glTF座標で(0,0,0)(1,0,0)(0,1,0))を、`translation`だけ動かしたノードに置く。
    /// `with_normals`がfalseなら法線なし。`base_color`はマテリアルの基本色。
    pub(crate) fn triangle_glb(translation: [f32; 3], with_normals: bool, base_color: [f32; 4]) -> Vec<u8> {
        let positions: [[f32; 3]; 3] = [[0.0, 0.0, 0.0], [1.0, 0.0, 0.0], [0.0, 1.0, 0.0]];
        let normals: [[f32; 3]; 3] = [[0.0, 0.0, 1.0]; 3];
        let mut bin: Vec<u8> = Vec::new();
        for p in positions {
            for v in p {
                bin.extend(v.to_le_bytes());
            }
        }
        let normals_offset = bin.len();
        if with_normals {
            for n in normals {
                for v in n {
                    bin.extend(v.to_le_bytes());
                }
            }
        }
        let mut views = vec![format!(r#"{{"buffer":0,"byteOffset":0,"byteLength":36}}"#)];
        let mut accessors = vec![
            r#"{"bufferView":0,"componentType":5126,"count":3,"type":"VEC3","min":[0,0,0],"max":[1,1,0]}"#.to_string(),
        ];
        let mut attributes = r#""POSITION":0"#.to_string();
        if with_normals {
            views.push(format!(r#"{{"buffer":0,"byteOffset":{normals_offset},"byteLength":36}}"#));
            accessors.push(r#"{"bufferView":1,"componentType":5126,"count":3,"type":"VEC3"}"#.to_string());
            attributes.push_str(r#","NORMAL":1"#);
        }
        let json = format!(
            r#"{{"asset":{{"version":"2.0"}},"scene":0,"scenes":[{{"nodes":[0]}}],
            "nodes":[{{"mesh":0,"translation":[{},{},{}]}}],
            "meshes":[{{"primitives":[{{"attributes":{{{attributes}}},"material":0}}]}}],
            "materials":[{{"pbrMetallicRoughness":{{"baseColorFactor":[{},{},{},{}]}}}}],
            "buffers":[{{"byteLength":{}}}],"bufferViews":[{}],"accessors":[{}]}}"#,
            translation[0],
            translation[1],
            translation[2],
            base_color[0],
            base_color[1],
            base_color[2],
            base_color[3],
            bin.len(),
            views.join(","),
            accessors.join(",")
        );
        let mut json_bytes = json.into_bytes();
        while json_bytes.len() % 4 != 0 {
            json_bytes.push(b' ');
        }
        while bin.len() % 4 != 0 {
            bin.push(0);
        }
        let total = 12 + 8 + json_bytes.len() + 8 + bin.len();
        let mut glb = Vec::with_capacity(total);
        glb.extend(b"glTF");
        glb.extend(2u32.to_le_bytes());
        glb.extend((total as u32).to_le_bytes());
        glb.extend((json_bytes.len() as u32).to_le_bytes());
        glb.extend(b"JSON");
        glb.extend(&json_bytes);
        glb.extend((bin.len() as u32).to_le_bytes());
        glb.extend(b"BIN\0");
        glb.extend(&bin);
        glb
    }

    #[test]
    fn imports_a_triangle_and_converts_axes_to_the_body_frame() {
        let mesh = import_glb(&triangle_glb([0.0, 0.0, 0.0], true, [1.0; 4])).unwrap();
        assert_eq!((mesh.vertices.len(), mesh.indices.len()), (3, 3));
        // glTF(1,0,0)=左(+X)は機体座標で右が-x、glTF(0,1,0)=上は機体座標のz。法線glTF+Z(前)は機体のy。
        assert_eq!(mesh.vertices[1].position, [-1.0, 0.0, 0.0]);
        assert_eq!(mesh.vertices[2].position, [0.0, 0.0, 1.0]);
        assert_eq!(mesh.vertices[0].normal, [0.0, 1.0, 0.0]);
        assert_eq!(mesh.indices, [0, 1, 2]);
        assert!((mesh.radius_m - 1.0).abs() < 1e-6);
    }

    #[test]
    fn node_transform_and_base_color_are_baked_into_the_vertices() {
        let mesh = import_glb(&triangle_glb([0.0, 0.0, 10.0], true, [0.5, 0.25, 1.0, 0.3])).unwrap();
        // ノードの平行移動(glTFの+Z=前へ10m)が頂点に入る。
        assert_eq!(mesh.vertices[0].position, [0.0, 10.0, 0.0]);
        // 基本色が頂点色に掛かる(アルファは常に1)。
        assert_eq!(mesh.vertices[0].color, [0.5, 0.25, 1.0, 1.0]);
    }

    #[test]
    fn missing_normals_become_flat_face_normals_with_split_vertices() {
        let mesh = import_glb(&triangle_glb([0.0; 3], false, [1.0; 4])).unwrap();
        assert_eq!(mesh.vertices.len(), 3);
        // 面の法線: (1,0,0)×(0,1,0)の外積=+Z(glTF)→機体のy。
        for v in &mesh.vertices {
            assert_eq!(v.normal, [0.0, 1.0, 0.0]);
        }
    }

    /// サンプルアプリのモデル(`scripts/gen_sample_models.py`が生成する)が読めて、実寸の大きさで、
    /// 三角形の向きが頂点の法線と合っている(裏返っていない)こと。
    #[test]
    fn sample_models_import_with_real_sizes_and_consistent_winding() {
        // (ファイル, 前後y方向の長さの範囲m)
        let expected = [
            ("aircraft", 14.0..16.0),
            ("helicopter", 12.0..13.5),
            ("ship", 120.0..135.0),
            ("vehicle", 8.0..10.0),
            ("missile", 4.5..5.5),
        ];
        let dir = concat!(env!("CARGO_MANIFEST_DIR"), "/../sample/sim_frontend/assets/models/");
        for (name, length_range) in expected {
            let bytes = std::fs::read(format!("{dir}{name}.glb"))
                .unwrap_or_else(|e| panic!("{name}.glbを読めません(python scripts/gen_sample_models.pyで生成): {e}"));
            let mesh = import_glb(&bytes).unwrap_or_else(|e| panic!("{name}: {e}"));
            let ys = mesh.vertices.iter().map(|v| v.position[1]);
            let length = ys.clone().fold(f32::MIN, f32::max) - ys.fold(f32::MAX, f32::min);
            assert!(length_range.contains(&length), "{name}: 前後の長さ{length}m");
            assert!(mesh.radius_m > 1.0 && mesh.radius_m < 100.0, "{name}: radius={}", mesh.radius_m);
            // 法線は単位ベクトルで、各三角形の面の向き(右手系の外積)が3頂点の法線の平均と同じ側を向く。
            let mut mismatched = 0;
            for t in mesh.indices.chunks_exact(3) {
                let v = [t[0], t[1], t[2]].map(|i| mesh.vertices[i as usize]);
                for vertex in &v {
                    let n = Vec3::from(vertex.normal);
                    assert!((n.length() - 1.0).abs() < 1e-3, "{name}: 法線が単位長でない");
                }
                let face = (Vec3::from(v[1].position) - Vec3::from(v[0].position))
                    .cross(Vec3::from(v[2].position) - Vec3::from(v[0].position));
                let average = v.iter().map(|x| Vec3::from(x.normal)).sum::<Vec3>();
                if face.dot(average) < 0.0 {
                    mismatched += 1;
                }
            }
            assert_eq!(mismatched, 0, "{name}: 向きが法線と逆の三角形がある");
        }
    }

    #[test]
    fn invalid_input_is_an_error_not_a_panic() {
        assert!(import_glb(b"not a glb").is_err());
        assert!(import_glb(&[]).is_err());
        let mut glb = triangle_glb([0.0; 3], true, [1.0; 4]);
        glb.truncate(glb.len() - 20); // バイナリチャンクが途中で切れている
        assert!(import_glb(&glb).is_err());
    }
}
