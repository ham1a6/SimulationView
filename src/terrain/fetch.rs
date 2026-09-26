//! 地形データのHTTP取得。データの形式・置き場所は`terrain::loader`のモジュール説明を参照。
//! 起動時に`metadata.json`・`tile_index.json`・`base.bin`(全タイルの最粗レベル)を取得し(`load_terrain`)、
//! 細かいレベルはカメラに近いチャンクだけ、必要に応じて取得する(`fetch_tile_level`・`fetch_chunk_grid`。
//! `terrain::lod`が必要なレベルを決め、`ui::terrain_view`が呼ぶ)。
//!
//! 取得の方針(DETAILED_DESIGN.md 9.1節・9.2節):
//! - 失敗はすべて`Err(String)`で返し、ここでは再試行しない(再試行するかどうかは呼び出し側が決める。
//!   細かいレベルの再試行は`ui::terrain_view::lod_driver`がバックオフを挟んで行う)。
//! - 受け取ったバイト数は必ず期待値と照合し、合わなければエラーにする(途中で切れた応答や、
//!   別のデータセットのファイルを黙って使わないため)。
//! - 通信の圧縮(gzip)はブラウザが自動で展開するので、ここで扱うバイト列は常に展開後のもの。

use std::cell::Cell;

use serde::de::DeserializeOwned;

use super::loader::{grid_len, TerrainData, TerrainMetadata, TileIndex, TileKey};

/// `{base_url}/{file}`のJSONを取得して`T`へデシリアライズする。
/// 通信の失敗は`"{file} fetch failed: …"`、JSONとして読めない・型が合わない場合は
/// `"{file} parse failed: …"`のエラーになる(HTTPのステータスは見ないので、404のHTMLなどは
/// 解析の失敗として報告される)。
async fn fetch_json<T: DeserializeOwned>(base_url: &str, file: &str) -> Result<T, String> {
    gloo_net::http::Request::get(&format!("{base_url}/{file}"))
        .send()
        .await
        .map_err(|e| format!("{file} fetch failed: {e}"))?
        .json()
        .await
        .map_err(|e| format!("{file} parse failed: {e}"))
}

/// リトルエンディアンの`int16`の並び(グリッドのファイル形式。DETAILED_DESIGN.md 9.1節)を
/// `i16`の配列にする。長さが奇数なら最後の1バイトは捨てる(呼び出し側がバイト数を先に
/// 検証しているので、実際には端数は出ない)。
fn decode_i16_le(bytes: &[u8]) -> Vec<i16> {
    bytes
        .as_chunks::<2>()
        .0
        .iter()
        .map(|c| i16::from_le_bytes(*c))
        .collect()
}

/// GETを送り、成功(2xx)の応答を返す。`range`は両端を含むバイト範囲(`Some((0, 9))`なら先頭10バイト)。
/// 2xx以外は`"{url} fetch failed: HTTP {status}"`のエラーにする(本文は読まない)。
async fn send_get(
    url: &str,
    range: Option<(usize, usize)>,
) -> Result<gloo_net::http::Response, String> {
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
    Ok(response)
}

/// `url`をバイト列として取得する。`range`を指定すると、その範囲(両端を含む)だけを返す。
///
/// Range対応のサーバーは`206 Partial Content`で指定範囲だけを返すが、未対応のサーバーは
/// `200`で全体を返す。その場合もここで必要な範囲を切り出すので、呼び出し側はどちらのサーバーでも
/// 同じ結果を受け取れる(転送量が増えるだけ)。3Dモデル(GLB)など地形以外の取得にも使う。
pub(crate) async fn fetch_binary(
    url: &str,
    range: Option<(usize, usize)>,
) -> Result<Vec<u8>, String> {
    let response = send_get(url, range).await?;
    // 本文を読むと応答は消費されるので、ステータスは先に控えておく。
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

/// `url`の全体を、本文のストリームを少しずつ読みながら取得する。読むたびに、それまでに受け取った
/// (展開後の)バイト数を`on_received`へ渡す。`Content-Length`は圧縮(gzip)後のサイズで展開後の
/// 大きさと一致しないので、全体の大きさは呼び出し側が別に知っている前提にする(`load_terrain`参照)。
async fn fetch_binary_streamed(
    url: &str,
    mut on_received: impl FnMut(usize),
) -> Result<Vec<u8>, String> {
    use wasm_bindgen::JsCast;

    let response = send_get(url, None).await?;
    let Some(stream) = response.body() else {
        // 本文のストリームが無い(空の応答)なら、まとめて読むのと同じ。
        return response
            .binary()
            .await
            .map_err(|e| format!("{url} read failed: {e}"));
    };
    let read_failed = |e: wasm_bindgen::JsValue| format!("{url} read failed: {e:?}");
    // `gloo_net`はストリームを少しずつ読むAPIを持たないので、Fetch APIの`ReadableStream`を
    // `web_sys`で直接読む。`read()`は、次の断片(`Uint8Array`)か終わり(`done: true`)を返すPromise。
    let reader = web_sys::ReadableStreamDefaultReader::new(&stream).map_err(read_failed)?;
    let mut body = Vec::new();
    loop {
        let chunk: web_sys::ReadableStreamReadResult =
            wasm_bindgen_futures::JsFuture::from(reader.read())
                .await
                .map_err(read_failed)?
                .unchecked_into();
        if chunk.get_done().unwrap_or(false) {
            return Ok(body);
        }
        // 受け取った断片を末尾へ追記する(JS側の配列からWASMのメモリへ1回だけコピーする)。
        let bytes = js_sys::Uint8Array::new(&chunk.get_value());
        let start = body.len();
        body.resize(start + bytes.length() as usize, 0);
        bytes.copy_to(&mut body[start..]);
        on_received(body.len());
    }
}

/// 起動時の取得(`load_terrain`)の進み具合。`TerrainStore::progress`で読む。
#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct TerrainLoadProgress {
    /// `base.bin`のうち受け取った(展開後の)バイト数。
    pub received_bytes: usize,
    /// `base.bin`全体の大きさ。`metadata.json`と`tile_index.json`が届くまでは分からないので`None`。
    pub total_bytes: Option<usize>,
}

impl TerrainLoadProgress {
    /// 0.0〜1.0の割合。全体の大きさがまだ分からなければ`None`。
    pub fn fraction(&self) -> Option<f64> {
        let total = self.total_bytes?;
        if total == 0 {
            return Some(1.0);
        }
        Some((self.received_bytes as f64 / total as f64).min(1.0))
    }
}

/// `base.bin`の総バイト数(`タイル数 × (N₀+1)² × 2`。DETAILED_DESIGN.md 9.1節)。
fn base_bin_len(metadata: &TerrainMetadata, index: &TileIndex) -> Option<usize> {
    let n0 = *metadata.tile_levels.first()? as usize;
    Some(index.tile_count() * grid_len(n0) * 2)
}

/// 起動時の取得: metadata.json・tile_index.json・base.bin(全タイルの最粗レベル)。
/// 3つとも他の結果に依存せず取得できる(検証にだけ互いの値を使う)ので、高遅延回線での
/// ラウンドトリップを減らすため同時に発行して待つ。
/// 進み具合は、容量のほとんどを占める`base.bin`の受信バイト数として`on_progress`へ通知する
/// (全体の大きさは先に届く`metadata.json`・`tile_index.json`から計算する)。
/// `base_url`はスキーム+ホスト+ルートパス(例: `"http://localhost:9001/terrain"`)。
/// 末尾にスラッシュを付けない。
pub async fn load_terrain(
    base_url: &str,
    on_progress: impl Fn(TerrainLoadProgress),
) -> Result<TerrainData, String> {
    // `format!`の一時値は`join!`の中で借用し続けるので、先に変数へ束縛して寿命を延ばす。
    let base_bin_url = format!("{base_url}/base.bin");
    // join!の各futureは同じタスク内で交互に進むだけなので、Cellで共有してよい。
    let received_bytes = Cell::new(0);
    let total_bytes = Cell::new(None);
    // 受信バイト数・全体の大きさのどちらかが変わるたびに、今の値をまとめて通知する。
    let report = || {
        on_progress(TerrainLoadProgress {
            received_bytes: received_bytes.get(),
            total_bytes: total_bytes.get(),
        })
    };
    // 外側の`join!`で「JSON 2つ」と「base.bin」を同時に進め、内側の`join!`でJSON 2つも同時に取る
    // (3つのリクエストがほぼ同時に出る)。JSONが揃った時点で`base.bin`の全体の大きさが分かるので、
    // その時点で一度通知しておく(進捗表示が「大きさ不明」から割合表示へ切り替わる)。
    let (metadata_and_index, bytes) = futures_util::join!(
        async {
            let (metadata, index) = futures_util::join!(
                fetch_json::<TerrainMetadata>(base_url, "metadata.json"),
                fetch_json::<TileIndex>(base_url, "tile_index.json"),
            );
            if let (Ok(metadata), Ok(index)) = (&metadata, &index) {
                total_bytes.set(base_bin_len(metadata, index));
                report();
            }
            (metadata, index)
        },
        fetch_binary_streamed(&base_bin_url, |received| {
            received_bytes.set(received);
            report();
        }),
    );
    // 3つとも待ち終えてから、エラーは metadata → tile_index → base.bin の順に報告する
    // (どれか1つの失敗で他の取得を途中で止めることはしない)。
    let (metadata, index) = metadata_and_index;
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

    // `base.bin`はタイル一覧の順にレベル0のグリッドを連結しただけで、区切りの情報を持たない。
    // 大きさが合わないとタイルとグリッドの対応がずれるので、ここで必ず弾く。
    // (`tile_levels`が空でないことは直前の検証で保証済みなので、`unwrap_or_default`は実際には0にならない。)
    let expected_len = base_bin_len(&metadata, &index).unwrap_or_default();
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
///
/// 返すのはチャンク番号順に連結したままの`i16`の並びで、チャンクごとに切り分けて登録するのは
/// `TerrainData::insert_tile_level`。小さいレベル(`loader::WHOLE_FILE_MAX_LEVEL`以下)で使い、
/// 1回のリクエストでタイルの全チャンクをまとめて得る(リクエスト数を減らすため)。
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
///
/// タイルファイルは固定サイズのレコード(チャンク1個分のグリッド)をチャンク番号順に並べたものなので、
/// `chunk`番目のレコードの位置は計算だけで決まる(索引は要らない。DETAILED_DESIGN.md 9.1節)。
/// 細かいレベル(`loader::WHOLE_FILE_MAX_LEVEL`より上)はタイルファイル全体が大きいので、
/// 画面に必要なチャンクだけをこの関数で取る。
pub async fn fetch_chunk_grid(
    terrain: &TerrainData,
    key: TileKey,
    level: usize,
    chunk: usize,
) -> Result<Vec<i16>, String> {
    let url = tile_level_url(&terrain.base_url, key, level);
    // レコード1個 = (n+1)²ノード × 2バイト(n = このレベルのチャンク1辺のセル数)。
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
    fn progress_fraction_needs_total_and_is_clamped() {
        let progress = |received_bytes, total_bytes| TerrainLoadProgress {
            received_bytes,
            total_bytes,
        };
        assert_eq!(progress(100, None).fraction(), None);
        assert_eq!(progress(0, Some(200)).fraction(), Some(0.0));
        assert_eq!(progress(50, Some(200)).fraction(), Some(0.25));
        // 展開後のサイズが想定より大きい応答(後でサイズ検証のエラーになる)でも100%を超えない。
        assert_eq!(progress(300, Some(200)).fraction(), Some(1.0));
        assert_eq!(progress(0, Some(0)).fraction(), Some(1.0));
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
