//! 地形データ(heightmap.bin / metadata.json)の取得。DETAILED_DESIGN.md 2.5節・2.6節。
//! サーバーが`{base_url}/metadata.json`・`{base_url}/heightmap.bin`でHTTP静的配信している
//! 前提(サンプルの`sample/sim_server`は`/terrain/*`ルートで配信する)。どのURLから取得するかは
//! ライブラリ側では決めず、呼び出し側が`base_url`として渡す(サーバーのホスト名・ポート・
//! ルートパスはアプリごとに異なりうるため)。

use serde::Deserialize;

#[derive(Debug, Clone, Copy, Deserialize)]
pub struct GeodeticBounds {
    pub min_lat: f64,
    pub max_lat: f64,
    pub min_lon: f64,
    pub max_lon: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct Ellipsoid {
    pub a_m: f64,
    pub inv_f: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct DefaultOrigin {
    pub lat_deg: f64,
    pub lon_deg: f64,
}

#[derive(Debug, Clone, Deserialize)]
pub struct TerrainMetadata {
    pub width: u32,
    pub height: u32,
    pub elevation_min: f32,
    pub elevation_max: f32,
    pub geodetic_bounds: GeodeticBounds,
    pub ellipsoid: Ellipsoid,
    #[allow(dead_code)] // v1では常にfalse。texture.png読み込み分岐を実装する際に使う。
    pub has_texture: bool,
    pub default_origin: DefaultOrigin,
}

pub struct TerrainData {
    pub metadata: TerrainMetadata,
    /// row-major, f32。行順は南→北(`geodetic_bounds`の座標復元式に対応。DETAILED_DESIGN.md 2.6節)。
    pub heightmap: Vec<f32>,
}

/// metadata.jsonだけを取得する(heightmap.binは取得しない、軽量版)。
/// 原点入力フォームのバリデーション(geodetic_bounds)用に使う。
/// `base_url`はスキーム+ホスト+ルートパス(例: `"http://localhost:9001/terrain"`)。
/// 末尾にスラッシュを付けない。
pub async fn fetch_metadata(base_url: &str) -> Result<TerrainMetadata, String> {
    gloo_net::http::Request::get(&format!("{base_url}/metadata.json"))
        .send()
        .await
        .map_err(|e| format!("metadata.json fetch failed: {e}"))?
        .json()
        .await
        .map_err(|e| format!("metadata.json parse failed: {e}"))
}

pub async fn load_terrain(base_url: &str) -> Result<TerrainData, String> {
    let metadata = fetch_metadata(base_url).await?;

    let bytes = gloo_net::http::Request::get(&format!("{base_url}/heightmap.bin"))
        .send()
        .await
        .map_err(|e| format!("heightmap.bin fetch failed: {e}"))?
        .binary()
        .await
        .map_err(|e| format!("heightmap.bin read failed: {e}"))?;

    let expected_len = metadata.width as usize * metadata.height as usize * 4;
    if bytes.len() != expected_len {
        return Err(format!(
            "heightmap.bin size mismatch: got {} bytes, expected {}",
            bytes.len(),
            expected_len
        ));
    }

    let mut heightmap = Vec::with_capacity(metadata.width as usize * metadata.height as usize);
    for chunk in bytes.chunks_exact(4) {
        heightmap.push(f32::from_le_bytes([chunk[0], chunk[1], chunk[2], chunk[3]]));
    }

    Ok(TerrainData { metadata, heightmap })
}
