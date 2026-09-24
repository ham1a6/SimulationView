//! 地形データの取得と保持(タイル+チャンク方式)。DETAILED_DESIGN.md 2.5節・2.6節。
//! (HTTPでの取得は`terrain::fetch`。ここは取得したデータの保持とサンプリング。)
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

use super::geodesy::Ellipsoid;

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
pub(super) struct TileIndex {
    tiles: Vec<TileIndexEntry>,
}

impl TileIndex {
    /// 存在するタイルの数。
    pub(super) fn tile_count(&self) -> usize {
        self.tiles.len()
    }
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
    pub(super) fn new(
        metadata: TerrainMetadata,
        base_url: String,
        index: TileIndex,
        base: Vec<i16>,
    ) -> Self {
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

    /// `detail`の添字。レベル0(タイル全体。チャンクを持たない)や範囲外のチャンクにはNone。
    fn slot(&self, level: usize, chunk: usize) -> Option<usize> {
        (level >= 1 && chunk < self.chunk_count()).then(|| (level - 1) * self.chunk_count() + chunk)
    }

    /// 指定レベル(1以上)・チャンクのグリッド。未取得ならNone。
    pub fn chunk_grid(&self, tile: &TileEntry, level: usize, chunk: usize) -> Option<Rc<Vec<i16>>> {
        let detail = tile.detail.borrow();
        let cached = detail.get(self.slot(level, chunk)?)?.as_ref()?;
        self.stamp.set(self.stamp.get() + 1);
        cached.last_used.set(self.stamp.get());
        Some(cached.data.clone())
    }

    pub fn has_chunk_grid(&self, tile: &TileEntry, level: usize, chunk: usize) -> bool {
        self.slot(level, chunk)
            .and_then(|slot| tile.detail.borrow().get(slot).map(|g| g.is_some()))
            .unwrap_or(false)
    }

    /// このチャンクについて、取得済みで`max_level`以下の最も細かいレベル(1以上)。無ければ0。
    pub fn best_cached_level(&self, tile: &TileEntry, chunk: usize, max_level: usize) -> usize {
        (1..=max_level.min(self.num_levels() - 1))
            .rev()
            .find(|&level| self.has_chunk_grid(tile, level, chunk))
            .unwrap_or(0)
    }

    /// 取得したチャンクグリッド(レベル1以上)を登録する。タイル・レベル・チャンクが範囲外か、
    /// 長さが`(チャンク1辺のセル数+1)^2`でなければ無視する(標高サンプリングが範囲外を引かないように)。
    pub fn insert_chunk_grid(&self, key: TileKey, level: usize, chunk: usize, data: Vec<i16>) {
        let Some(tile) = self.tile(key) else { return };
        if level >= self.num_levels() {
            return;
        }
        let Some(slot) = self.slot(level, chunk) else {
            return;
        };
        let nodes = self.chunk_cells(level) + 1;
        if data.len() != nodes * nodes {
            return;
        }
        self.stamp.set(self.stamp.get() + 1);
        let bytes = data.len() * std::mem::size_of::<i16>();
        let mut detail = tile.detail.borrow_mut();
        let slot = &mut detail[slot];
        if slot.is_none() {
            self.cached_bytes.set(self.cached_bytes.get() + bytes);
        }
        *slot = Some(CachedGrid {
            data: Rc::new(data),
            last_used: Cell::new(self.stamp.get()),
        });
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
            let (v00, v10, v01, v11) = (
                at(j0, i0),
                at(j0, i0 + 1),
                at(j0 + 1, i0),
                at(j0 + 1, i0 + 1),
            );
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
            if let Some(Some(cached)) = self
                .slot(level, cy * k + cx)
                .and_then(|slot| detail.get(slot))
            {
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
        bilinear(
            self.whole_grid(tile),
            cells,
            u * cells as f64,
            v * cells as f64,
        )
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
                self.cached_bytes.set(
                    self.cached_bytes
                        .get()
                        .saturating_sub(g.data.len() * std::mem::size_of::<i16>()),
                );
            }
        }
    }
}

#[cfg(test)]
impl TerrainData {
    /// 単体テスト用の合成地形。`min_lat`/`min_lon`から`rows`x`cols`枚の1度タイルを並べ、
    /// レベル0をすべて`height(緯度, 経度)`(メートル)で埋める。既定のレベルは6・12・24セル/度
    /// (チャンク分割は2x2)で、細かいレベル(1以上)は空。必要なら`insert_chunk_grid`で足す。
    pub(crate) fn synthetic(
        min_lat: i32,
        min_lon: i32,
        rows: i32,
        cols: i32,
        height: impl Fn(f64, f64) -> i16,
    ) -> Self {
        Self::synthetic_with_levels(min_lat, min_lon, rows, cols, vec![6, 12, 24], 2, height)
    }

    /// `synthetic`のレベル定義を指定できる版。レベル0のグリッドだけを作るので、LODの計画
    /// (`terrain::lod`)のように実際のレベルの値(実データは`[60, 180, 600, 1800, 3600]`・6分割)を
    /// 使いたいテストでも軽い。
    pub(crate) fn synthetic_with_levels(
        min_lat: i32,
        min_lon: i32,
        rows: i32,
        cols: i32,
        tile_levels: Vec<u32>,
        chunks_per_tile: u32,
        height: impl Fn(f64, f64) -> i16,
    ) -> Self {
        let metadata = TerrainMetadata {
            tile_levels,
            chunks_per_tile,
            elevation_min: -100.0,
            elevation_max: 4000.0,
            geodetic_bounds: GeodeticBounds {
                min_lat: min_lat as f64,
                max_lat: (min_lat + rows) as f64,
                min_lon: min_lon as f64,
                max_lon: (min_lon + cols) as f64,
            },
            ellipsoid: Ellipsoid::WGS84,
            has_texture: false,
            default_origin: DefaultOrigin {
                lat_deg: min_lat as f64 + 0.5,
                lon_deg: min_lon as f64 + 0.5,
            },
        };
        let cells0 = metadata.tile_levels[0] as usize;
        let mut entries = Vec::new();
        let mut base = Vec::new();
        for lat in min_lat..min_lat + rows {
            for lon in min_lon..min_lon + cols {
                let (mut lo, mut hi) = (f32::MAX, f32::MIN);
                for j in 0..=cells0 {
                    for i in 0..=cells0 {
                        let h = height(
                            lat as f64 + j as f64 / cells0 as f64,
                            lon as f64 + i as f64 / cells0 as f64,
                        );
                        if h != NO_DATA {
                            lo = lo.min(h as f32);
                            hi = hi.max(h as f32);
                        }
                        base.push(h);
                    }
                }
                entries.push(TileIndexEntry {
                    lat,
                    lon,
                    elevation_min: lo,
                    elevation_max: hi,
                });
            }
        }
        Self::new(
            metadata,
            "http://test".to_string(),
            TileIndex { tiles: entries },
            base,
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: TileKey = (30, 120);

    /// タイル(30,120)内で、東へ1度で600m・北へ1度で60m上がる斜面(ノード間隔が1/6度なので整数)。
    fn slope(lat: f64, lon: f64) -> i16 {
        (600.0 * (lon - 120.0) + 60.0 * (lat - 30.0)).round() as i16
    }

    fn one_tile(height: impl Fn(f64, f64) -> i16) -> TerrainData {
        TerrainData::synthetic(30, 120, 1, 1, height)
    }

    #[test]
    fn tile_lookup_uses_bounds() {
        let data = TerrainData::synthetic(30, 120, 2, 3, |_, _| 0);
        assert_eq!(data.tiles().len(), 6);
        assert!(data.tile((31, 122)).is_some());
        assert!(data.tile((32, 120)).is_none());
        assert!(data.tile((29, 120)).is_none());
        assert!(data.tile((30, 123)).is_none());
    }

    #[test]
    fn bilinear_reproduces_a_linear_slope_on_the_base_grid() {
        let data = one_tile(slope);
        let tile = data.tile(KEY).unwrap();
        assert!((data.sample_bilinear(tile, 0.25, 0.5) - 180.0).abs() < 1e-3);
        assert!((data.sample_bilinear(tile, 0.0, 0.0)).abs() < 1e-3);
        // 東端・北端(u,v=1)でも範囲外を引かずに端の値を返す。
        assert!((data.sample_bilinear(tile, 1.0, 1.0) - 660.0).abs() < 1e-3);
    }

    #[test]
    fn bilinear_treats_no_data_neighbours_as_sea_level() {
        let data = one_tile(|lat, lon| {
            if lon > 120.5 {
                NO_DATA
            } else {
                slope(lat, lon)
            }
        });
        let tile = data.tile(KEY).unwrap();
        assert!(data.sample_bilinear(tile, 0.2, 0.5) > 0.0);
        assert_eq!(data.sample_bilinear(tile, 0.9, 0.5), 0.0);
    }

    #[test]
    fn detail_grid_is_used_only_after_the_chunk_level_is_set() {
        let data = one_tile(|_, _| 100);
        let tile = data.tile(KEY).unwrap();
        let nodes = data.chunk_cells(1) + 1;
        data.insert_chunk_grid(KEY, 1, 0, vec![500; nodes * nodes]);
        assert!(data.has_chunk_grid(tile, 1, 0));
        // 登録しただけでは、まだ画面に出していないレベルなのでサンプリングは変わらない。
        assert_eq!(data.sample_bilinear(tile, 0.1, 0.1), 100.0);
        data.set_chunk_level(KEY, 0, 1);
        assert_eq!(data.sample_bilinear(tile, 0.1, 0.1), 500.0);
        // 別のチャンク(東側)はレベル0のまま。
        assert_eq!(data.sample_bilinear(tile, 0.9, 0.1), 100.0);
        data.set_whole_tile(KEY);
        assert_eq!(data.sample_bilinear(tile, 0.1, 0.1), 100.0);
    }

    #[test]
    fn insert_chunk_grid_ignores_bad_arguments() {
        let data = one_tile(|_, _| 0);
        let tile = data.tile(KEY).unwrap();
        let nodes = data.chunk_cells(1) + 1;
        data.insert_chunk_grid(KEY, 1, 0, vec![0; nodes * nodes - 1]); // 長さ不一致
        data.insert_chunk_grid(KEY, 0, 0, vec![0; nodes * nodes]); // レベル0はチャンクを持たない
        data.insert_chunk_grid(KEY, 1, data.chunk_count(), vec![0; nodes * nodes]); // チャンク範囲外
        data.insert_chunk_grid(KEY, data.num_levels(), 0, vec![0; nodes * nodes]); // レベル範囲外
        data.insert_chunk_grid((0, 0), 1, 0, vec![0; nodes * nodes]); // タイル無し
        assert_eq!(data.cached_bytes(), 0);
        assert!(!data.has_chunk_grid(tile, 1, 0));
        // レベル0やチャンク範囲外の問い合わせでもパニックしない。
        assert!(data.chunk_grid(tile, 0, 0).is_none());
        assert!(!data.has_chunk_grid(tile, 0, 0));
        assert!(!data.has_chunk_grid(tile, 1, data.chunk_count()));
    }

    #[test]
    fn insert_tile_level_splits_records_per_chunk() {
        let data = one_tile(|_, _| 0);
        let tile = data.tile(KEY).unwrap();
        let nodes = data.chunk_cells(1) + 1;
        let record = nodes * nodes;
        let mut all = Vec::new();
        for chunk in 0..data.chunk_count() {
            all.extend(std::iter::repeat_n(chunk as i16 + 1, record));
        }
        data.insert_tile_level(KEY, 1, &all);
        for chunk in 0..data.chunk_count() {
            assert_eq!(
                data.chunk_grid(tile, 1, chunk).unwrap()[0],
                chunk as i16 + 1
            );
        }
        assert_eq!(data.cached_bytes(), data.chunk_count() * record * 2);
        assert_eq!(data.best_cached_level(tile, 0, 2), 1);
        assert_eq!(data.best_cached_level(tile, 0, 0), 0);
    }

    #[test]
    fn evict_unused_drops_oldest_first_and_respects_keep() {
        let data = one_tile(|_, _| 0);
        let tile = data.tile(KEY).unwrap();
        let nodes = data.chunk_cells(1) + 1;
        let one = nodes * nodes * 2;
        for chunk in 0..3 {
            data.insert_chunk_grid(KEY, 1, chunk, vec![0; nodes * nodes]);
        }
        assert_eq!(data.cached_bytes(), 3 * one);
        // チャンク0を使い直して、最後に使われたのを新しくする。
        data.chunk_grid(tile, 1, 0);

        // 上限内なら何もしない。
        data.evict_unused(|_, _, _| false, 3 * one);
        assert_eq!(data.cached_bytes(), 3 * one);

        // チャンク1(いちばん古い)だけが消える。
        data.evict_unused(|_, _, _| false, 2 * one);
        assert!(!data.has_chunk_grid(tile, 1, 1));
        assert!(data.has_chunk_grid(tile, 1, 0) && data.has_chunk_grid(tile, 1, 2));

        // keepが真のものは、上限を超えていても残す。
        data.evict_unused(|_, chunk, _| chunk == 2, 0);
        assert!(data.has_chunk_grid(tile, 1, 2));
        assert!(!data.has_chunk_grid(tile, 1, 0));
        assert_eq!(data.cached_bytes(), one);
    }
}
