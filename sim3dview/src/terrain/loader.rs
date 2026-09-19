//! 地形データの取得と保持(タイル+チャンク方式)。DETAILED_DESIGN.md 2.5節・2.6節。
//! サーバーが`{base_url}/`以下でHTTP静的配信している前提(サンプルの`sample/sim_server`は
//! `/terrain/*`ルートで配信する)。どのURLから取得するかはライブラリ側では決めず、呼び出し側が
//! `base_url`として渡す(サーバーのホスト名・ポート・ルートパスはアプリごとに異なりうるため)。
//!
//! データは1度x1度のタイル単位で、タイルごとに複数の解像度レベルを持つ(`geotiff_preprocess`が
//! 生成する。フォーマットはそのツールのヘッダコメント参照):
//! - `metadata.json` ... 全体の範囲・標高範囲・楕円体・レベル定義(`tile_levels`)・チャンク分割数
//! - `tile_index.json` ... 存在するタイルの一覧
//! - `base.bin` ... レベル0(最粗。タイル全体で1枚)を全タイル分連結したもの。起動時に全部取得して
//!   常時保持する
//! - `tiles/L{k}/N035E138.bin` ... レベルk(1以上)のタイル別ファイル。1度タイルを
//!   `chunks_per_tile`x`chunks_per_tile`のチャンクに分けた、チャンクごとのグリッドを連結したもの。
//!   カメラに近いチャンクだけ必要に応じて取得する(`terrain::lod`が必要なレベルを決め、
//!   `ui::terrain_view`が取得する)。レベル`WHOLE_FILE_MAX_LEVEL`以下はファイルが小さいので
//!   まとめて取得し、それより細かいレベルはHTTP Rangeでチャンク1個分だけ取得する
//!
//! グリッドは(N+1)x(N+1)ノードのint16(メートル)で、行は南→北、列は西→東。データなしは
//! `NO_DATA`(int16の最小値)。

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use serde::Deserialize;

/// タイルの識別子: 南西角の(緯度, 経度)(整数度)。
pub type TileKey = (i32, i32);

/// GPU上のメッシュの識別子: (タイル緯度, タイル経度, チャンク番号)。チャンク番号が
/// `WHOLE_TILE`ならタイル全体(レベル0)を1枚のメッシュで持つ。
pub type MeshKey = (i32, i32, u8);

/// `MeshKey`のチャンク番号として、タイル全体(レベル0)のメッシュを表す値。
pub const WHOLE_TILE: u8 = u8::MAX;

/// グリッド上の「データなし(海など)」を表す値。
pub const NO_DATA: i16 = i16::MIN;

/// このレベル以下は、タイル1枚分(全チャンク)のファイルをまとめて取得する(小さいので1回で済ませる)。
/// これより細かいレベルはファイルが大きいので、HTTP Rangeでチャンク1個分だけ取得する。
pub const WHOLE_FILE_MAX_LEVEL: usize = 2;

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
    /// 解像度レベルごとの、1度タイル1辺あたりのセル数(先頭がレベル0=最粗、以降ほど細かい)。
    /// レベル1以上は`chunks_per_tile`で割り切れる。
    pub tile_levels: Vec<u32>,
    /// 1度タイルを何x何のチャンクに分けるか(レベル1以上)。
    pub chunks_per_tile: u32,
    pub elevation_min: f32,
    pub elevation_max: f32,
    pub geodetic_bounds: GeodeticBounds,
    pub ellipsoid: Ellipsoid,
    #[allow(dead_code)] // v1では常にfalse。texture.png読み込み分岐を実装する際に使う。
    pub has_texture: bool,
    pub default_origin: DefaultOrigin,
}

#[derive(Debug, Deserialize)]
struct TileIndex {
    tiles: Vec<TileIndexEntry>,
}

#[derive(Debug, Deserialize)]
struct TileIndexEntry {
    lat: i32,
    lon: i32,
    elevation_min: f32,
    elevation_max: f32,
}

/// 取得済みの細かいレベルのチャンクグリッド1枚。`last_used`は解放(`TerrainData::evict_unused`)の
/// 優先順位に使う。
struct CachedGrid {
    data: Rc<Vec<i16>>,
    last_used: Cell<u64>,
}

/// 1タイル分の状態。`chunk_level`は「いま画面に出している(=標高サンプリングにも使う)レベル」で、
/// 描画されているメッシュと標高の問い合わせ(観測点・見通し計算・クリック判定)を一致させる。
pub struct TileEntry {
    pub key: TileKey,
    pub elevation_min: f32,
    pub elevation_max: f32,
    base_offset: usize,
    /// チャンク(行(南→北)*分割数+列(西→東))ごとの、いま画面に出しているレベル。
    /// 0ならタイル全体をレベル0で出している。
    chunk_level: Vec<Cell<u8>>,
    /// 添字は(レベル-1)*チャンク数+チャンク番号。
    detail: RefCell<Vec<Option<CachedGrid>>>,
}

pub struct TerrainData {
    pub metadata: TerrainMetadata,
    /// 細かいレベルのグリッドを取得する際のベースURL。
    pub base_url: String,
    base: Vec<i16>,
    tiles: Vec<TileEntry>,
    /// (緯度-最小緯度)*cols+(経度-最小経度) → `tiles`の添字。
    slots: Vec<Option<usize>>,
    cols: i32,
    stamp: Cell<u64>,
    cached_bytes: Cell<usize>,
}

impl TerrainData {
    fn new(metadata: TerrainMetadata, base_url: String, index: TileIndex, base: Vec<i16>) -> Self {
        let b = metadata.geodetic_bounds;
        let rows = (b.max_lat - b.min_lat).round() as i32;
        let cols = (b.max_lon - b.min_lon).round() as i32;
        let nodes0 = metadata.tile_levels[0] as usize + 1;
        let grid0 = nodes0 * nodes0;
        let chunks = (metadata.chunks_per_tile * metadata.chunks_per_tile) as usize;
        let num_detail = metadata.tile_levels.len() - 1;

        let mut tiles = Vec::with_capacity(index.tiles.len());
        let mut slots = vec![None; (rows.max(0) * cols.max(0)) as usize];
        for (i, entry) in index.tiles.iter().enumerate() {
            let key = (entry.lat, entry.lon);
            let row = entry.lat - b.min_lat as i32;
            let col = entry.lon - b.min_lon as i32;
            if row >= 0 && row < rows && col >= 0 && col < cols {
                slots[(row * cols + col) as usize] = Some(i);
            }
            tiles.push(TileEntry {
                key,
                elevation_min: entry.elevation_min,
                elevation_max: entry.elevation_max,
                base_offset: i * grid0,
                chunk_level: (0..chunks).map(|_| Cell::new(0)).collect(),
                detail: RefCell::new((0..num_detail * chunks).map(|_| None).collect()),
            });
        }
        Self {
            metadata,
            base_url,
            base,
            tiles,
            slots,
            cols,
            stamp: Cell::new(0),
            cached_bytes: Cell::new(0),
        }
    }

    pub fn tiles(&self) -> &[TileEntry] {
        &self.tiles
    }

    pub fn tile(&self, key: TileKey) -> Option<&TileEntry> {
        let b = &self.metadata.geodetic_bounds;
        let row = key.0 - b.min_lat as i32;
        let col = key.1 - b.min_lon as i32;
        if row < 0 || col < 0 || col >= self.cols {
            return None;
        }
        let slot = *self.slots.get((row * self.cols + col) as usize)?;
        slot.map(|i| &self.tiles[i])
    }

    pub fn num_levels(&self) -> usize {
        self.metadata.tile_levels.len()
    }

    /// 1度タイルの一辺を何チャンクに分けるか。
    pub fn chunks_per_tile(&self) -> usize {
        self.metadata.chunks_per_tile as usize
    }

    /// 1度タイルあたりのチャンク数。
    pub fn chunk_count(&self) -> usize {
        self.chunks_per_tile() * self.chunks_per_tile()
    }

    /// レベルkの、1度タイル1辺あたりのセル数。
    pub fn level_cells(&self, level: usize) -> usize {
        self.metadata.tile_levels[level] as usize
    }

    /// レベルk(1以上)の、チャンク1辺あたりのセル数(ノード数はこれ+1)。
    pub fn chunk_cells(&self, level: usize) -> usize {
        self.level_cells(level) / self.chunks_per_tile()
    }

    /// タイル全体のグリッド(レベル0。常にある)。
    pub fn whole_grid(&self, tile: &TileEntry) -> &[i16] {
        let n = self.level_cells(0) + 1;
        &self.base[tile.base_offset..tile.base_offset + n * n]
    }

    fn slot(&self, level: usize, chunk: usize) -> usize {
        (level - 1) * self.chunk_count() + chunk
    }

    /// 指定レベル(1以上)・チャンクのグリッド。未取得ならNone。
    pub fn chunk_grid(&self, tile: &TileEntry, level: usize, chunk: usize) -> Option<Rc<Vec<i16>>> {
        let detail = tile.detail.borrow();
        let cached = detail.get(self.slot(level, chunk))?.as_ref()?;
        self.stamp.set(self.stamp.get() + 1);
        cached.last_used.set(self.stamp.get());
        Some(cached.data.clone())
    }

    pub fn has_chunk_grid(&self, tile: &TileEntry, level: usize, chunk: usize) -> bool {
        tile.detail.borrow().get(self.slot(level, chunk)).is_some_and(|g| g.is_some())
    }

    /// このチャンクについて、取得済みで`max_level`以下の最も細かいレベル(1以上)。無ければ0。
    pub fn best_cached_level(&self, tile: &TileEntry, chunk: usize, max_level: usize) -> usize {
        (1..=max_level.min(self.num_levels() - 1))
            .rev()
            .find(|&level| self.has_chunk_grid(tile, level, chunk))
            .unwrap_or(0)
    }

    /// 取得したチャンクグリッド(レベル1以上)を登録する。
    pub fn insert_chunk_grid(&self, key: TileKey, level: usize, chunk: usize, data: Vec<i16>) {
        let Some(tile) = self.tile(key) else { return };
        if level == 0 || level >= self.num_levels() || chunk >= self.chunk_count() {
            return;
        }
        self.stamp.set(self.stamp.get() + 1);
        let bytes = data.len() * std::mem::size_of::<i16>();
        let mut detail = tile.detail.borrow_mut();
        let slot = &mut detail[self.slot(level, chunk)];
        if slot.is_none() {
            self.cached_bytes.set(self.cached_bytes.get() + bytes);
        }
        *slot = Some(CachedGrid { data: Rc::new(data), last_used: Cell::new(self.stamp.get()) });
    }

    /// タイル1枚分・1レベルのファイル(全チャンクのレコードを連結したもの)を、チャンクごとに
    /// 分けて登録する。
    pub fn insert_tile_level(&self, key: TileKey, level: usize, all: &[i16]) {
        let nodes = self.chunk_cells(level) + 1;
        let record = nodes * nodes;
        for chunk in 0..self.chunk_count() {
            if let Some(slice) = all.get(chunk * record..(chunk + 1) * record) {
                self.insert_chunk_grid(key, level, chunk, slice.to_vec());
            }
        }
    }

    /// このチャンクの、画面に出しているレベルを記録する(標高サンプリングと描画を一致させる)。
    pub fn set_chunk_level(&self, key: TileKey, chunk: usize, level: usize) {
        if let Some(tile) = self.tile(key) {
            if let Some(cell) = tile.chunk_level.get(chunk) {
                cell.set(level as u8);
            }
        }
    }

    /// タイル全体をレベル0で出している状態にする(全チャンクのレベルを0にする)。
    pub fn set_whole_tile(&self, key: TileKey) {
        if let Some(tile) = self.tile(key) {
            for cell in &tile.chunk_level {
                cell.set(0);
            }
        }
    }

    /// タイル内の位置(u=経度方向, v=緯度方向。ともに0〜1)の標高を、いま画面に出している
    /// レベルのグリッドから双線形補間で求める。周辺4ノードのいずれかがデータなし(海)なら0。
    pub fn sample_bilinear(&self, tile: &TileEntry, u: f64, v: f64) -> f32 {
        let k = self.chunks_per_tile();
        let cx = ((u * k as f64).floor().max(0.0) as usize).min(k - 1);
        let cy = ((v * k as f64).floor().max(0.0) as usize).min(k - 1);
        let level = tile.chunk_level[cy * k + cx].get() as usize;

        // (グリッドの参照, 一辺のセル数, グリッド内の連続座標fx/fy)を決める。
        let bilinear = |grid: &[i16], cells: usize, fx: f64, fy: f64| -> f32 {
            let nodes = cells + 1;
            let i0 = (fx.floor().max(0.0) as usize).min(cells - 1);
            let j0 = (fy.floor().max(0.0) as usize).min(cells - 1);
            let tx = (fx - i0 as f64).clamp(0.0, 1.0) as f32;
            let ty = (fy - j0 as f64).clamp(0.0, 1.0) as f32;
            let at = |j: usize, i: usize| grid[j * nodes + i];
            let (v00, v10, v01, v11) =
                (at(j0, i0), at(j0, i0 + 1), at(j0 + 1, i0), at(j0 + 1, i0 + 1));
            if v00 == NO_DATA || v10 == NO_DATA || v01 == NO_DATA || v11 == NO_DATA {
                return 0.0;
            }
            let (h00, h10, h01, h11) = (v00 as f32, v10 as f32, v01 as f32, v11 as f32);
            let h0 = h00 + (h10 - h00) * tx;
            let h1 = h01 + (h11 - h01) * tx;
            h0 + (h1 - h0) * ty
        };

        if level > 0 {
            let detail = tile.detail.borrow();
            if let Some(Some(cached)) = detail.get(self.slot(level, cy * k + cx)) {
                let n_level = self.level_cells(level) as f64;
                let cells = self.chunk_cells(level);
                return bilinear(
                    &cached.data,
                    cells,
                    u * n_level - (cx * cells) as f64,
                    v * n_level - (cy * cells) as f64,
                );
            }
        }
        let cells = self.level_cells(0);
        bilinear(self.whole_grid(tile), cells, u * cells as f64, v * cells as f64)
    }

    /// 取得済みの細かいレベルのグリッドの合計バイト数。
    pub fn cached_bytes(&self) -> usize {
        self.cached_bytes.get()
    }

    /// 合計が`limit_bytes`以下になるまで、`keep(タイル, チャンク, レベル)`がfalseを返すグリッド
    /// (=いま画面に出していないもの)を、最後に使われたのが古い順に破棄する。
    pub fn evict_unused(&self, keep: impl Fn(TileKey, usize, usize) -> bool, limit_bytes: usize) {
        if self.cached_bytes.get() <= limit_bytes {
            return;
        }
        let chunks = self.chunk_count();
        let mut candidates: Vec<(u64, usize, usize)> = Vec::new(); // (last_used, タイルidx, スロット)
        for (ti, tile) in self.tiles.iter().enumerate() {
            for (si, slot) in tile.detail.borrow().iter().enumerate() {
                if let Some(g) = slot {
                    let (level, chunk) = (si / chunks + 1, si % chunks);
                    if !keep(tile.key, chunk, level) {
                        candidates.push((g.last_used.get(), ti, si));
                    }
                }
            }
        }
        candidates.sort_unstable();
        for (_, ti, si) in candidates {
            if self.cached_bytes.get() <= limit_bytes {
                break;
            }
            let mut detail = self.tiles[ti].detail.borrow_mut();
            if let Some(g) = detail[si].take() {
                self.cached_bytes
                    .set(self.cached_bytes.get().saturating_sub(g.data.len() * 2));
            }
        }
    }
}

/// metadata.jsonだけを取得する(タイル一覧・ベースは取得しない、軽量版)。
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

fn decode_i16_le(bytes: &[u8]) -> Vec<i16> {
    bytes.chunks_exact(2).map(|c| i16::from_le_bytes([c[0], c[1]])).collect()
}

async fn fetch_binary(url: &str, range: Option<(usize, usize)>) -> Result<Vec<u8>, String> {
    let mut request = gloo_net::http::Request::get(url);
    if let Some((start, end)) = range {
        // Rangeは単純な`bytes=start-end`ならCORSのプリフライトなしで送れる。
        request = request.header("Range", &format!("bytes={start}-{end}"));
    }
    let response = request.send().await.map_err(|e| format!("{url} fetch failed: {e}"))?;
    if !response.ok() {
        return Err(format!("{url} fetch failed: HTTP {}", response.status()));
    }
    let status = response.status();
    let body = response.binary().await.map_err(|e| format!("{url} read failed: {e}"))?;
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
pub async fn load_terrain(base_url: &str) -> Result<TerrainData, String> {
    let metadata = fetch_metadata(base_url).await?;
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

    let index: TileIndex = gloo_net::http::Request::get(&format!("{base_url}/tile_index.json"))
        .send()
        .await
        .map_err(|e| format!("tile_index.json fetch failed: {e}"))?
        .json()
        .await
        .map_err(|e| format!("tile_index.json parse failed: {e}"))?;

    let bytes = fetch_binary(&format!("{base_url}/base.bin"), None).await?;
    let nodes0 = metadata.tile_levels[0] as usize + 1;
    let expected_len = index.tiles.len() * nodes0 * nodes0 * 2;
    if bytes.len() != expected_len {
        return Err(format!(
            "base.bin size mismatch: got {} bytes, expected {}",
            bytes.len(),
            expected_len
        ));
    }
    Ok(TerrainData::new(metadata, base_url.to_string(), index, decode_i16_le(&bytes)))
}

/// タイル名("N035E138"形式。`geotiff_preprocess`の出力ファイル名と一致させる)。
pub fn tile_name(key: TileKey) -> String {
    format!(
        "{}{:03}{}{:03}",
        if key.0 >= 0 { 'N' } else { 'S' },
        key.0.abs(),
        if key.1 >= 0 { 'E' } else { 'W' },
        key.1.abs()
    )
}

/// タイル1枚分・1レベル(1以上)のファイル(全チャンクのレコードを連結したもの)を取得する。
/// サイズが期待と違えばエラー。
pub async fn fetch_tile_level(
    base_url: &str,
    key: TileKey,
    level: usize,
    chunk_cells: usize,
    chunk_count: usize,
) -> Result<Vec<i16>, String> {
    let url = format!("{base_url}/tiles/L{level}/{}.bin", tile_name(key));
    let bytes = fetch_binary(&url, None).await?;
    let expected = chunk_count * (chunk_cells + 1) * (chunk_cells + 1) * 2;
    if bytes.len() != expected {
        return Err(format!("{url} size mismatch: got {} bytes, expected {expected}", bytes.len()));
    }
    Ok(decode_i16_le(&bytes))
}

/// チャンク1個分のグリッドを、タイルファイルからHTTP Rangeで取得する。
pub async fn fetch_chunk_grid(
    base_url: &str,
    key: TileKey,
    level: usize,
    chunk: usize,
    chunk_cells: usize,
) -> Result<Vec<i16>, String> {
    let url = format!("{base_url}/tiles/L{level}/{}.bin", tile_name(key));
    let record_bytes = (chunk_cells + 1) * (chunk_cells + 1) * 2;
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
