//! 地形データのHTTP取得。データの形式・置き場所は`terrain::loader`のモジュール説明を参照。
//! 起動時に`metadata.json`・`tile_index.json`・`base.bin`(全タイルの最粗レベル)を取得し(`load_terrain`)、
//! 細かいレベルはカメラに近いチャンクだけ、必要に応じて取得する(`fetch_tile_level`・`fetch_chunk_grid`。
//! `terrain::lod`が必要なレベルを決め、`ui::terrain_view`が呼ぶ)。

use serde::de::DeserializeOwned;

use super::loader::{grid_len, TerrainData, TerrainMetadata, TileIndex, TileKey};

/// `{base_url}/{file}`のJSONを取得する。
async fn fetch_json<T: DeserializeOwned>(base_url: &str, file: &str) -> Result<T, String> {
    gloo_net::http::Request::get(&format!("{base_url}/{file}"))
        .send()
        .await
        .map_err(|e| format!("{file} fetch failed: {e}"))?
        .json()
        .await
        .map_err(|e| format!("{file} parse failed: {e}"))
}

fn decode_i16_le(bytes: &[u8]) -> Vec<i16> {
    bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| i16::from_le_bytes(*c))
        .collect()
}

pub(crate) async fn fetch_binary(
    url: &str,
    range: Option<(usize, usize)>,
) -> Result<Vec<u8>, String> {
    let mut request = gloo_net::http::Request::get(url);
    if let Some((start, end)) = range {
        // Rangeは単純な`bytes=start-end`ならCORSのプリフライトなしで送れる。
        request = request.header("Range", &format!("bytes={start}-{end}"));
    }
    let response = request
        .send()
        .await
        .map_err(|e| format!("{url} fetch failed: {e}"))?;
    if !response.ok() {
        return Err(format!("{url} fetch failed: HTTP {}", response.status()));
    }
    let status = response.status();
    let body = response
        .binary()
        .await
        .map_err(|e| format!("{url} read failed: {e}"))?;
    match range {
        // サーバーがRangeを無視して全体(200)を返した場合は、必要な範囲を切り出す。
        Some((start, end)) if status == 200 => body
            .get(start..=end)
            .map(|s| s.to_vec())
            .ok_or_else(|| format!("{url}: range {start}-{end} is out of the response body")),
        _ => Ok(body),
    }
}

/// 起動時の取得: metadata.json・tile_index.json・base.bin(全タイルの最粗レベル)。
/// 3つとも他の結果に依存せず取得できる(検証にだけ互いの値を使う)ので、高遅延回線での
/// ラウンドトリップを減らすため同時に発行して待つ。
/// `base_url`はスキーム+ホスト+ルートパス(例: `"http://localhost:9001/terrain"`)。
/// 末尾にスラッシュを付けない。
pub async fn load_terrain(base_url: &str) -> Result<TerrainData, String> {
    let base_bin_url = format!("{base_url}/base.bin");
    let (metadata, index, bytes) = futures_util::join!(
        fetch_json::<TerrainMetadata>(base_url, "metadata.json"),
        fetch_json::<TileIndex>(base_url, "tile_index.json"),
        fetch_binary(&base_bin_url, None),
    );
    let metadata = metadata?;
    let index = index?;
    let bytes = bytes?;

    // レベル0(タイル全体)に加えて、チャンクで持つ細かいレベルが1つ以上必要(LODの計画が前提にする)。
    // レベル1以上はチャンク分割数で割り切れること。
    let chunks = metadata.chunks_per_tile;
    if metadata.tile_levels.len() < 2
        || chunks == 0
        || metadata.tile_levels[1..].iter().any(|&n| n % chunks != 0)
    {
        return Err(
            "metadata.json: tile_levels needs 2+ levels, and levels 1+ must be divisible by chunks_per_tile"
                .to_string(),
        );
    }

    let expected_len = index.tile_count() * grid_len(metadata.tile_levels[0] as usize) * 2;
    if bytes.len() != expected_len {
        return Err(format!(
            "base.bin size mismatch: got {} bytes, expected {}",
            bytes.len(),
            expected_len
        ));
    }
    Ok(TerrainData::new(
        metadata,
        base_url.to_string(),
        index,
        decode_i16_le(&bytes),
    ))
}

/// タイル名("N035E138"形式。`geotiff_preprocess`の出力ファイル名と一致させる)。
fn tile_name(key: TileKey) -> String {
    format!(
        "{}{:03}{}{:03}",
        if key.0 >= 0 { 'N' } else { 'S' },
        key.0.abs(),
        if key.1 >= 0 { 'E' } else { 'W' },
        key.1.abs()
    )
}

/// タイル1枚・1レベル(1以上)のファイルのURL(`{base_url}/tiles/L{level}/N035E138.bin`)。
fn tile_level_url(base_url: &str, key: TileKey, level: usize) -> String {
    format!("{base_url}/tiles/L{level}/{}.bin", tile_name(key))
}

/// タイル1枚分・1レベル(1以上)のファイル(全チャンクのレコードを連結したもの)を、
/// `terrain.base_url`から取得する。サイズが期待と違えばエラー。
pub async fn fetch_tile_level(
    terrain: &TerrainData,
    key: TileKey,
    level: usize,
) -> Result<Vec<i16>, String> {
    let url = tile_level_url(&terrain.base_url, key, level);
    let bytes = fetch_binary(&url, None).await?;
    let expected = terrain.chunk_count() * grid_len(terrain.chunk_cells(level)) * 2;
    if bytes.len() != expected {
        return Err(format!(
            "{url} size mismatch: got {} bytes, expected {expected}",
            bytes.len()
        ));
    }
    Ok(decode_i16_le(&bytes))
}

/// チャンク1個分のグリッドを、タイルファイルからHTTP Rangeで取得する。
pub async fn fetch_chunk_grid(
    terrain: &TerrainData,
    key: TileKey,
    level: usize,
    chunk: usize,
) -> Result<Vec<i16>, String> {
    let url = tile_level_url(&terrain.base_url, key, level);
    let record_bytes = grid_len(terrain.chunk_cells(level)) * 2;
    let start = chunk * record_bytes;
    let bytes = fetch_binary(&url, Some((start, start + record_bytes - 1))).await?;
    if bytes.len() != record_bytes {
        return Err(format!(
            "{url} chunk {chunk} size mismatch: got {} bytes, expected {record_bytes}",
            bytes.len()
        ));
    }
    Ok(decode_i16_le(&bytes))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn tile_name_uses_hemisphere_letters() {
        assert_eq!(tile_name((35, 138)), "N035E138");
        assert_eq!(tile_name((-1, -5)), "S001W005");
        assert_eq!(tile_name((0, 0)), "N000E000");
    }

    #[test]
    fn decode_reads_little_endian_i16() {
        assert_eq!(
            decode_i16_le(&[0x34, 0x12, 0xff, 0xff, 0x00, 0x80]),
            vec![0x1234, -1, i16::MIN]
        );
        // 端数のバイトは捨てる。
        assert_eq!(decode_i16_le(&[1, 0, 9]), vec![1]);
    }
}
