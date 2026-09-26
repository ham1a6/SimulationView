//! 3Dモデル(`terrain::models`)のGPU側: モデルごとの頂点・インデックスと、インスタンス(1機ごとの変換行列)のバッファ、
//! 描画パイプライン(`model.wgsl`)。モデルの追加・削除・インスタンスの更新は`TerrainRenderer`の`set_model`等が、
//! 描画は`encode_main_pass`が呼ぶ。パイプラインは頂点バッファ2本(頂点・インスタンス)を使う。

use std::collections::HashMap;

use wgpu::util::DeviceExt;

use super::pipelines::{
    create_pipeline, pipeline_layout, vertex_layout, wgsl_module, PipelineSpec,
};
use super::targets::SAMPLE_COUNT;
use super::uniforms::UniformSlot;
use crate::terrain::models::types::{ModelInstance, ModelMesh, ModelVertex};

/// 頂点(`ModelVertex`)のシェーダー入力(`model.wgsl`の`VertexInput`のlocation 0〜2)。
pub(super) const MODEL_VERTEX_ATTRIBUTES: [wgpu::VertexAttribute; 3] =
    wgpu::vertex_attr_array![0 => Float32x3, 1 => Float32x3, 2 => Float32x4];
/// インスタンス(`ModelInstance`)のシェーダー入力(location 3〜7。行列の4列と色)。
pub(super) const MODEL_INSTANCE_ATTRIBUTES: [wgpu::VertexAttribute; 5] = wgpu::vertex_attr_array![3 => Float32x4, 4 => Float32x4, 5 => Float32x4, 6 => Float32x4, 7 => Float32x4];

/// インスタンスバッファの最小の容量(個)。
const MIN_INSTANCE_CAPACITY: usize = 16;

/// モデル1つ分のGPUバッファ。
struct ModelGpu {
    /// `ModelVertex`の並び。
    vertex_buffer: wgpu::Buffer,
    /// 三角形の頂点番号(u32)。
    index_buffer: wgpu::Buffer,
    /// インデックスの数。
    num_indices: u32,
    /// インスタンスのバッファ(容量は`capacity`個。足りなくなったら大きく作り直す)。
    instance_buffer: Option<wgpu::Buffer>,
    /// `instance_buffer`に入るインスタンスの数(0ならバッファ未作成)。
    capacity: usize,
    /// このフレームに描くインスタンスの数。
    count: u32,
}

/// 登録済みの全モデルと、その描画パイプライン。
pub(super) struct ModelBatch {
    pipeline: wgpu::RenderPipeline,
    /// モデルのキー(URL)→GPUバッファ。
    models: HashMap<String, ModelGpu>,
}

impl ModelBatch {
    /// パイプラインを作る(モデルは空)。uniformは作図の絶対座標用と同じレイアウトを使う。
    pub(super) fn new(
        device: &wgpu::Device,
        format: wgpu::TextureFormat,
        draw_bind_group_layout: &wgpu::BindGroupLayout,
    ) -> Self {
        let shader = wgsl_module(device, "model_shader", include_str!("../models/model.wgsl"));
        let layout = pipeline_layout(device, "model_pipeline_layout", &[draw_bind_group_layout]);
        let buffers = [
            Some(vertex_layout::<ModelVertex>(
                &MODEL_VERTEX_ATTRIBUTES,
                wgpu::VertexStepMode::Vertex,
            )),
            Some(vertex_layout::<ModelInstance>(
                &MODEL_INSTANCE_ATTRIBUTES,
                wgpu::VertexStepMode::Instance,
            )),
        ];
        // 裏面カリングは使わない(両面のモデルがある。`create_pipeline`の共通の設定)。
        let pipeline = create_pipeline(
            device,
            &PipelineSpec {
                label: "model_pipeline",
                layout: &layout,
                shader: &shader,
                vs_entry: "vs_main",
                fs_entry: "fs_main",
                buffers: &buffers,
                format,
                blend: wgpu::BlendState::REPLACE,
                // 不透明なので深度を書く。反転Zなので「より近ければ描く」はGreater。
                depth: Some((true, wgpu::CompareFunction::Greater)),
                samples: SAMPLE_COUNT,
            },
        );
        Self {
            pipeline,
            models: HashMap::new(),
        }
    }

    /// モデルを登録する(同じキーがあれば置き換える。インスタンスは空になる)。
    pub(super) fn set_model(&mut self, device: &wgpu::Device, key: &str, mesh: &ModelMesh) {
        if mesh.indices.is_empty() {
            self.models.remove(key);
            return;
        }
        let vertex_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("model_vertex_buffer"),
            contents: bytemuck::cast_slice(&mesh.vertices),
            usage: wgpu::BufferUsages::VERTEX,
        });
        let index_buffer = device.create_buffer_init(&wgpu::util::BufferInitDescriptor {
            label: Some("model_index_buffer"),
            contents: bytemuck::cast_slice(&mesh.indices),
            usage: wgpu::BufferUsages::INDEX,
        });
        self.models.insert(
            key.to_string(),
            ModelGpu {
                vertex_buffer,
                index_buffer,
                num_indices: mesh.indices.len() as u32,
                instance_buffer: None,
                capacity: 0,
                count: 0,
            },
        );
    }

    /// モデルの登録を外す(GPUバッファは解放される)。
    pub(super) fn remove_model(&mut self, key: &str) {
        self.models.remove(key);
    }

    /// このフレームに描くインスタンスを差し替える。`instances`に無いモデルは、何も描かない。登録されていないキーは無視する。
    pub(super) fn set_instances(
        &mut self,
        device: &wgpu::Device,
        queue: &wgpu::Queue,
        instances: &HashMap<String, Vec<ModelInstance>>,
    ) {
        for (key, model) in self.models.iter_mut() {
            let list: &[ModelInstance] = instances.get(key).map_or(&[], |v| v.as_slice());
            model.count = list.len() as u32;
            if list.is_empty() {
                continue;
            }
            // 容量が足りなければ、2のべき乗(最小`MIN_INSTANCE_CAPACITY`)に切り上げて作り直す。
            if list.len() > model.capacity {
                model.capacity = list.len().next_power_of_two().max(MIN_INSTANCE_CAPACITY);
                model.instance_buffer = Some(device.create_buffer(&wgpu::BufferDescriptor {
                    label: Some("model_instance_buffer"),
                    size: (model.capacity * std::mem::size_of::<ModelInstance>())
                        as wgpu::BufferAddress,
                    usage: wgpu::BufferUsages::VERTEX | wgpu::BufferUsages::COPY_DST,
                    mapped_at_creation: false,
                }));
            }
            if let Some(buffer) = model.instance_buffer.as_ref() {
                queue.write_buffer(buffer, 0, bytemuck::cast_slice(list));
            }
        }
    }

    /// 描くインスタンスが1つも無いか。
    fn is_empty(&self) -> bool {
        self.models.values().all(|m| m.count == 0)
    }

    /// インスタンスのあるモデルを、モデルごとに1回のインスタンス描画で描く。
    /// `space`は作図の絶対座標のuniform(`DrawUniform`)。
    pub(super) fn draw(&self, pass: &mut wgpu::RenderPass<'_>, space: &UniformSlot) {
        if self.is_empty() {
            return;
        }
        pass.set_pipeline(&self.pipeline);
        pass.set_bind_group(0, space.bind_group(), &[]);
        for model in self.models.values().filter(|m| m.count > 0) {
            let Some(instances) = model.instance_buffer.as_ref() else {
                continue;
            };
            pass.set_vertex_buffer(0, model.vertex_buffer.slice(..));
            pass.set_vertex_buffer(1, instances.slice(..));
            pass.set_index_buffer(model.index_buffer.slice(..), wgpu::IndexFormat::Uint32);
            pass.draw_indexed(0..model.num_indices, 0, 0..model.count);
        }
    }
}
