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
//!
//! 保持の仕組み(DETAILED_DESIGN.md 9.2節):
//! - レベル0は`base`に全タイル分を常駐させる(どのタイルも最低限この解像度で描ける)。
//! - レベル1以上はチャンク単位で`TileEntry::detail`に置き、合計バイト数が上限を超えたら
//!   画面に出していないものから古い順に捨てる(`evict_unused`)。
//! - 各チャンクの「いま画面に出しているレベル」(`TileEntry::chunk_level`)を持ち、標高の問い合わせは
//!   必ずそのレベルのグリッドを使う。描画されている地形と、観測点・見通し計算・クリック位置などの
//!   標高が食い違わないようにするため。
//!
//! `TerrainData`は`Rc`で共有され、描画側と取得側の両方から触られるが、WASMのメインスレッドだけで
//! 使うので、可変な部分は`Cell`・`RefCell`で持つ(`&self`のままグリッドを追加・破棄できる)。

use std::cell::{Cell, RefCell};
use std::rc::Rc;

use serde::Deserialize;

use super::geodesy::Ellipsoid;
use super::origin::Origin;

/// タイルの識別子: 南西角の(緯度, 経度)(整数度)。
pub type TileKey = (i32, i32);

/// GPU上のメッシュの識別子: (タイル緯度, タイル経度, チャンク番号)。チャンク番号が
/// `WHOLE_TILE`ならタイル全体(レベル0)を1枚のメッシュで持つ。
pub type MeshKey = (i32, i32, u8);

/// `MeshKey`のチャンク番号として、タイル全体(レベル0)のメッシュを表す値。
pub const WHOLE_TILE: u8 = u8::MAX;

/// グリッド上の「データなし(海など)」を表す値。
pub const NO_DATA: i16 = i16::MIN;

/// 一辺`cells`セルのグリッドのノード数(`(cells+1)^2`)。
pub(crate) fn grid_len(cells: usize) -> usize {
    (cells + 1) * (cells + 1)
}

/// このレベル以下は、タイル1枚分(全チャンク)のファイルをまとめて取得する(小さいので1回で済ませる)。
/// これより細かいレベルはファイルが大きいので、HTTP Rangeでチャンク1個分だけ取得する。
pub const WHOLE_FILE_MAX_LEVEL: usize = 2;

/// 緯度経度の矩形(度)。地形データ全体の範囲(`metadata.json`の`geodetic_bounds`)や、
/// 地表の三角形を列挙する範囲の指定(`TerrainData::visit_surface_triangles`)に使う。
/// 地形データ全体の範囲は、前処理ツールが1度タイルの外接矩形として出すので四隅とも整数度になる。
#[derive(Debug, Clone, Copy, Deserialize)]
pub struct GeodeticBounds {
    /// 南端の緯度(度)。
    pub min_lat: f64,
    /// 北端の緯度(度)。
    pub max_lat: f64,
    /// 西端の経度(度)。
    pub min_lon: f64,
    /// 東端の経度(度)。
    pub max_lon: f64,
}

/// `metadata.json`の内容(DETAILED_DESIGN.md 2.6節・9.1節)。未知のフィールドは無視する。
/// 形の妥当性(レベルが2つ以上あるか、チャンク分割数で割り切れるか)は`fetch::load_terrain`が検証する。
#[derive(Debug, Clone, Deserialize)]
pub struct TerrainMetadata {
    /// 解像度レベルごとの、1度タイル1辺あたりのセル数(先頭がレベル0=最粗、以降ほど細かい)。
    /// レベル1以上は`chunks_per_tile`で割り切れる。
    pub tile_levels: Vec<u32>,
    /// 1度タイルを何x何のチャンクに分けるか(レベル1以上)。
    pub chunks_per_tile: u32,
    /// データ全体の最低標高(メートル)。元データの水面ノイズで大きな負の値になりうるため、
    /// 配色の下限には使わない(`mesh`は0mを下限にする)。
    pub elevation_min: f32,
    /// データ全体の最高標高(メートル)。配色(`mesh`のカラーランプ)の上端と、クリック位置を求める
    /// レイマーチで「これより上に地表は無い」とみなす高さ(`pick`)に使う。
    pub elevation_max: f32,
    /// データのある範囲(1度タイルの外接矩形)。タイルの添字計算(`TerrainData::tile`)の基準にもなる。
    pub geodetic_bounds: GeodeticBounds,
    /// 緯度経度の基準の楕円体(ALOSはGRS80)。ENU座標への変換に使う。
    /// なお標高は楕円体高ではなくジオイド(EGM96)基準の正標高のまま持っている。
    pub ellipsoid: Ellipsoid,
    #[allow(dead_code)] // v1では常にfalse。texture.png読み込み分岐を実装する際に使う。
    pub has_texture: bool,
    /// 呼び出し側が原点を指定しないときに使う原点(前処理ツールが固定値を書き出す)。
    pub default_origin: Origin,
}

/// `tile_index.json`の内容: 陸のある(ファイルが存在する)タイルの一覧。
/// 並び順(緯度昇順→経度昇順)が`base.bin`の中のタイルの並びと一致する(DETAILED_DESIGN.md 9.1節)。
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

/// `tile_index.json`の1タイル分。
#[derive(Debug, Deserialize)]
struct TileIndexEntry {
    /// タイル南西角の緯度(整数度)。
    lat: i32,
    /// タイル南西角の経度(整数度)。
    lon: i32,
    /// タイル内の実測の最低標高(メートル)。
    elevation_min: f32,
    /// タイル内の実測の最高標高(メートル)。
    elevation_max: f32,
}

/// 取得済みの細かいレベルのチャンクグリッド1枚。`last_used`は解放(`TerrainData::evict_unused`)の
/// 優先順位に使う。
struct CachedGrid {
    /// `(n+1)²`ノードの標高。メッシュ生成・標高サンプリングへ複製せずに渡せるよう`Rc`で持つ。
    data: Rc<Vec<i16>>,
    /// 最後に使われたときの`TerrainData::stamp`の値(大きいほど最近)。
    last_used: Cell<u64>,
}

impl CachedGrid {
    /// このグリッドが使うメモリのバイト数(`TerrainData::cached_bytes`の集計単位)。
    fn bytes(&self) -> usize {
        self.data.len() * std::mem::size_of::<i16>()
    }
}

/// 1タイル分の状態。`chunk_level`は「いま画面に出している(=標高サンプリングにも使う)レベル」で、
/// 描画されているメッシュと標高の問い合わせ(観測点・見通し計算・クリック判定)を一致させる。
pub struct TileEntry {
    /// タイルの南西角(緯度, 経度)。
    pub key: TileKey,
    /// タイル内の最低標高(メートル)。LODの距離計算では最低・最高の中間の高さを代表点に使う。
    pub elevation_min: f32,
    /// タイル内の最高標高(メートル)。
    pub elevation_max: f32,
    /// `TerrainData::base`の中で、このタイルのレベル0グリッドが始まる位置(要素数)。
    base_offset: usize,
    /// チャンク(行(南→北)*分割数+列(西→東))ごとの、いま画面に出しているレベル。
    /// 0ならタイル全体をレベル0で出している。
    chunk_level: Vec<Cell<u8>>,
    /// 添字は(レベル-1)*チャンク数+チャンク番号。
    detail: RefCell<Vec<Option<CachedGrid>>>,
}

impl TileEntry {
    /// チャンクの、いま画面に出しているレベル(0ならタイル全体をレベル0で出している)。
    fn level(&self, chunk: usize) -> usize {
        self.chunk_level[chunk].get() as usize
    }
}

/// 取得した地形データ全体(DETAILED_DESIGN.md 9.2節)。`fetch::load_terrain`が作り、`Rc`で
/// 地図・見通し計算・断面図などから共有する。タイルは緯度経度から定数時間で引ける(`tile`)。
pub struct TerrainData {
    /// `metadata.json`の内容。
    pub metadata: TerrainMetadata,
    /// 細かいレベルのグリッドを取得する際のベースURL。
    pub base_url: String,
    /// 全タイルのレベル0グリッドを`tile_index.json`の順に連結したもの(`base.bin`そのもの)。
    base: Vec<i16>,
    /// 存在するタイル(`tile_index.json`の順)。
    tiles: Vec<TileEntry>,
    /// (緯度-最小緯度)*cols+(経度-最小経度) → `tiles`の添字。
    /// データ範囲の全1度マスぶんを持ち、海だけのマス(タイルが無い)はNone。
    slots: Vec<Option<usize>>,
    /// データ範囲の東西方向のマス数(`slots`の1行の長さ)。
    cols: i32,
    /// グリッドが使われるたびに1ずつ増える通し番号(`CachedGrid::last_used`に記録してLRUに使う)。
    stamp: Cell<u64>,
    /// 取得済みの細かいレベルのグリッドの合計バイト数(`evict_unused`の判定に使う)。
    cached_bytes: Cell<usize>,
}

impl TerrainData {
    /// 取得した3つのファイルの内容から組み立てる。`base`の長さの検証は呼び出し側(`fetch::load_terrain`)が
    /// 済ませている前提。細かいレベルのグリッドは空で始まり、全チャンクがレベル0(タイル全体)を出している状態になる。
    pub(super) fn new(
        metadata: TerrainMetadata,
        base_url: String,
        index: TileIndex,
        base: Vec<i16>,
    ) -> Self {
        // 範囲は整数度のはずだが、JSONの浮動小数点の誤差に備えて丸めてからマス数にする。
        let b = metadata.geodetic_bounds;
        let rows = (b.max_lat - b.min_lat).round() as i32;
        let cols = (b.max_lon - b.min_lon).round() as i32;
        let grid0 = grid_len(metadata.tile_levels[0] as usize);
        let chunks = (metadata.chunks_per_tile * metadata.chunks_per_tile) as usize;
        let num_detail = metadata.tile_levels.len() - 1;

        let mut tiles = Vec::with_capacity(index.tiles.len());
        let mut slots = vec![None; (rows.max(0) * cols.max(0)) as usize];
        for (i, entry) in index.tiles.iter().enumerate() {
            let key = (entry.lat, entry.lon);
            let row = entry.lat - b.min_lat as i32;
            let col = entry.lon - b.min_lon as i32;
            // 範囲外のタイル(壊れた索引)は`tiles`には入れるが、緯度経度からは引けないようにする。
            if row >= 0 && row < rows && col >= 0 && col < cols {
                slots[(row * cols + col) as usize] = Some(i);
            }
            // レベル0のグリッドは索引の順に`base`へ並んでいるので、位置は順番×1枚の大きさで決まる。
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

    /// 存在する全タイル(`tile_index.json`の順)。
    pub fn tiles(&self) -> &[TileEntry] {
        &self.tiles
    }

    /// 指定範囲に重なる表示LODの地表三角形を、経度・緯度・標高で列挙する。
    /// 未取得・欠損・範囲外は海面を張る。スカートは地表ではないため含めない。
    ///
    /// 各頂点は`[経度, 緯度, 標高(メートル)]`。三角形の分け方は描画メッシュ(`mesh`)と同じ
    /// (セルの南東―北西の対角線で2つに分ける)なので、作図を地表へ貼り付けるとき
    /// (`drawing_geometry::drape`)に、画面に見えている地形とぴったり重ねられる。
    pub(crate) fn visit_surface_triangles(
        &self,
        bounds: GeodeticBounds,
        mut visit: impl FnMut([[f64; 3]; 3]),
    ) {
        // 範囲に掛かる1度マスを順に見る(タイルが無いマスも、海面として三角形を出す)。
        for lat in bounds.min_lat.floor() as i32..bounds.max_lat.ceil() as i32 {
            for lon in bounds.min_lon.floor() as i32..bounds.max_lon.ceil() as i32 {
                let tile = self.tile((lat, lon));
                // グリッド1枚(タイル全体のレベル0、またはチャンク1個)のうち、範囲に掛かるセルの三角形を出す。
                // `values`: 標高(Noneなら全ノード0m=海面)、`cells`: 一辺のセル数、`start`: 南西角の[経度, 緯度]、
                // `step`: ノード間隔(度)、`base`: レベル0のグリッドか(細かいレベルで表示中のチャンクを飛ばすため)。
                let mut grid = |values: Option<&[i16]>,
                                cells: usize,
                                start: [f64; 2],
                                step: f64,
                                base: bool| {
                    // 範囲に掛かるセルの添字範囲[begin, end)。範囲の端がセルの途中でも、そのセルは含める。
                    let begin = [
                        ((bounds.min_lon - start[0]) / step).floor().max(0.0) as usize,
                        ((bounds.min_lat - start[1]) / step).floor().max(0.0) as usize,
                    ];
                    let end = [
                        (((bounds.max_lon - start[0]) / step).ceil().max(0.0) as usize).min(cells),
                        (((bounds.max_lat - start[1]) / step).ceil().max(0.0) as usize).min(cells),
                    ];
                    for j in begin[1]..end[1] {
                        for i in begin[0]..end[0] {
                            // レベル0のセルでも、そのセルのチャンクが細かいレベルで表示中(グリッドも手元にある)なら、
                            // 画面の地表はそちらなので飛ばす(細かいレベルのほうは後で別に列挙する)。
                            if base
                                && tile.is_some_and(|t| {
                                    let chunk = self
                                        .chunk_at((i as f64 + 0.5) * step, (j as f64 + 0.5) * step);
                                    let level = t.level(chunk);
                                    level > 0 && self.chunk_grid(t, level, chunk).is_some()
                                })
                            {
                                continue;
                            }
                            // ノード(列x, 行y)の[経度, 緯度, 標高]。グリッドは行が南→北のrow-major。
                            let node = |x: usize, y: usize| {
                                [
                                    start[0] + x as f64 * step,
                                    start[1] + y as f64 * step,
                                    values.map_or(0, |g| g[y * (cells + 1) + x]) as f64,
                                ]
                            };
                            // a=南西、b=南東、c=北西、d=北東。対角線b―c(南東―北西)で2つの三角形に分ける
                            // (`mesh::grid_indices`と同じ分け方)。
                            let [a, b, c, d] = [
                                node(i, j),
                                node(i + 1, j),
                                node(i, j + 1),
                                node(i + 1, j + 1),
                            ];
                            for mut tri in [[a, b, c], [b, d, c]] {
                                // 描画メッシュはデータなしのノードを含む三角形を描かず、その下の水面
                                // (楕円体面)が見える。ここでもその三角形は丸ごと海面(0m)として扱う。
                                if tri.iter().any(|p| p[2] == NO_DATA as f64) {
                                    for p in &mut tri {
                                        p[2] = 0.0;
                                    }
                                }
                                visit(tri);
                            }
                        }
                    }
                };
                // まずレベル0(タイルが無ければ海面)を、細かいレベルで表示中のセルを除いて出す。
                let base_cells = self.level_cells(0);
                grid(
                    tile.map(|t| self.whole_grid(t)),
                    base_cells,
                    [lon as f64, lat as f64],
                    1.0 / base_cells as f64,
                    true,
                );
                // 次に、細かいレベルで表示中のチャンクを、そのレベルのグリッドで出す。
                if let Some(tile) = tile {
                    for chunk in 0..self.chunk_count() {
                        let level = tile.level(chunk);
                        if level == 0 {
                            continue;
                        }
                        if let Some(values) = self.chunk_grid(tile, level, chunk) {
                            let (lat_start, lon_start, step) =
                                self.chunk_placement(tile.key, level, chunk);
                            grid(
                                Some(&values),
                                self.chunk_cells(level),
                                [lon_start, lat_start],
                                step,
                                false,
                            );
                        }
                    }
                }
            }
        }
    }

    /// 南西角が`key`のタイル。データ範囲外や、海だけでタイルが無いマスはNone。
    pub fn tile(&self, key: TileKey) -> Option<&TileEntry> {
        let b = &self.metadata.geodetic_bounds;
        let row = key.0 - b.min_lat as i32;
        let col = key.1 - b.min_lon as i32;
        // 列は範囲を明示的に確かめる(はみ出すと隣の行に回り込むため)。行の上限は`slots.get`が弾く。
        if row < 0 || col < 0 || col >= self.cols {
            return None;
        }
        let slot = *self.slots.get((row * self.cols + col) as usize)?;
        slot.map(|i| &self.tiles[i])
    }

    /// 解像度レベルの数(レベル0を含む)。`fetch::load_terrain`の検証により2以上。
    pub fn num_levels(&self) -> usize {
        self.metadata.tile_levels.len()
    }

    /// 最も細かいレベル。
    pub fn max_level(&self) -> usize {
        self.num_levels() - 1
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

    /// レベル(1以上)のチャンクの南西角の(緯度, 経度)と、ノードの間隔(度)。
    pub(crate) fn chunk_placement(
        &self,
        key: TileKey,
        level: usize,
        chunk: usize,
    ) -> (f64, f64, f64) {
        let k = self.chunks_per_tile();
        let cells = self.chunk_cells(level);
        let step = 1.0 / self.level_cells(level) as f64;
        // チャンク番号 = 行(南→北)*k + 列(西→東)。行は緯度、列は経度の方向にチャンク1個分ずつずれる。
        (
            key.0 as f64 + (chunk / k * cells) as f64 * step,
            key.1 as f64 + (chunk % k * cells) as f64 * step,
            step,
        )
    }

    /// タイル内の位置(u=経度方向, v=緯度方向。ともに0〜1)を含むチャンクの番号(範囲外は端のチャンク)。
    fn chunk_at(&self, u: f64, v: f64) -> usize {
        let k = self.chunks_per_tile();
        let cx = ((u * k as f64).floor().max(0.0) as usize).min(k - 1);
        let cy = ((v * k as f64).floor().max(0.0) as usize).min(k - 1);
        cy * k + cx
    }

    /// タイル全体のグリッド(レベル0。常にある)。
    pub fn whole_grid(&self, tile: &TileEntry) -> &[i16] {
        &self.base[tile.base_offset..tile.base_offset + grid_len(self.level_cells(0))]
    }

    /// `detail`の添字。レベル0(タイル全体。チャンクを持たない)や範囲外のチャンクにはNone。
    fn slot(&self, level: usize, chunk: usize) -> Option<usize> {
        (level >= 1 && chunk < self.chunk_count()).then(|| (level - 1) * self.chunk_count() + chunk)
    }

    /// 指定レベル(1以上)・チャンクのグリッド。未取得ならNone。
    pub fn chunk_grid(&self, tile: &TileEntry, level: usize, chunk: usize) -> Option<Rc<Vec<i16>>> {
        let detail = tile.detail.borrow();
        let cached = detail.get(self.slot(level, chunk)?)?.as_ref()?;
        cached.last_used.set(self.next_stamp());
        Some(cached.data.clone())
    }

    /// 解放の優先順位に使う、使われた順の番号を1つ進めて返す。
    fn next_stamp(&self) -> u64 {
        let stamp = self.stamp.get() + 1;
        self.stamp.set(stamp);
        stamp
    }

    /// 指定レベル(1以上)・チャンクのグリッドを取得済みか。`chunk_grid`と違い、使われた順(LRU)は
    /// 更新しない(取得が要るかの確認だけで、解放の優先順位を変えないため)。
    pub fn has_chunk_grid(&self, tile: &TileEntry, level: usize, chunk: usize) -> bool {
        self.slot(level, chunk)
            .is_some_and(|slot| matches!(tile.detail.borrow().get(slot), Some(Some(_))))
    }

    /// このチャンクについて、取得済みで`max_level`以下の最も細かいレベル(1以上)。無ければ0。
    pub fn best_cached_level(&self, tile: &TileEntry, chunk: usize, max_level: usize) -> usize {
        (1..=max_level.min(self.max_level()))
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
        if data.len() != grid_len(self.chunk_cells(level)) {
            return;
        }
        let grid = CachedGrid {
            data: Rc::new(data),
            last_used: Cell::new(self.next_stamp()),
        };
        let mut detail = tile.detail.borrow_mut();
        let slot = &mut detail[slot];
        // 同じグリッドの取り直し(上書き)なら大きさは同じなので、合計には空きスロットへ入れたときだけ足す。
        if slot.is_none() {
            self.cached_bytes
                .set(self.cached_bytes.get() + grid.bytes());
        }
        *slot = Some(grid);
    }

    /// タイル1枚分・1レベルのファイル(全チャンクのレコードを連結したもの)を、チャンクごとに
    /// 分けて登録する。
    /// 長さが足りずレコードが欠けるチャンクは登録しない(取得側でファイル全体の長さを検証済み)。
    pub fn insert_tile_level(&self, key: TileKey, level: usize, all: &[i16]) {
        let record = grid_len(self.chunk_cells(level));
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
        self.sample_grid(tile, u, v, false)
    }

    /// 描画メッシュと同じ南東―北西の対角線で三角形補間する。
    pub(crate) fn sample_surface(&self, tile: &TileEntry, u: f64, v: f64) -> f32 {
        self.sample_grid(tile, u, v, true)
    }

    /// `sample_bilinear`(`surface=false`)と`sample_surface`(`surface=true`)の共通部分。
    /// 位置を含むチャンクの表示中のレベルのグリッドがあればそれを、無ければレベル0を使う。
    fn sample_grid(&self, tile: &TileEntry, u: f64, v: f64, surface: bool) -> f32 {
        let chunk = self.chunk_at(u, v);
        let level = tile.level(chunk);

        // (グリッドの参照, 一辺のセル数, グリッド内の連続座標fx/fy)を決める。
        let bilinear = |grid: &[i16], cells: usize, fx: f64, fy: f64| -> f32 {
            let nodes = cells + 1;
            // 位置を含むセルの南西ノード(i0, j0)。端(fx=cells)でも最後のセルに収め、範囲外を引かない。
            let i0 = (fx.floor().max(0.0) as usize).min(cells - 1);
            let j0 = (fy.floor().max(0.0) as usize).min(cells - 1);
            // セル内の位置(0〜1)。
            let tx = (fx - i0 as f64).clamp(0.0, 1.0) as f32;
            let ty = (fy - j0 as f64).clamp(0.0, 1.0) as f32;
            let at = |j: usize, i: usize| grid[j * nodes + i];
            let (v00, v10, v01, v11) = (
                at(j0, i0),
                at(j0, i0 + 1),
                at(j0 + 1, i0),
                at(j0 + 1, i0 + 1),
            );
            if surface {
                // 描画メッシュと同じ三角形(対角線は南東―北西)の上で、重心座標で補間する。
                // tx+ty<=1なら南西側の三角形(南西・南東・北西)、それ以外は北東側(北東・北西・南東)。
                let (values, weights) = if tx + ty <= 1.0 {
                    ([v00, v10, v01], [1.0 - tx - ty, tx, ty])
                } else {
                    ([v11, v01, v10], [tx + ty - 1.0, 1.0 - tx, 1.0 - ty])
                };
                // 海岸では、描画される三角形の3ノードだけで判定する。
                return if values.contains(&NO_DATA) {
                    0.0
                } else {
                    values.iter().zip(weights).map(|(&h, w)| h as f32 * w).sum()
                };
            }
            // 双線形補間では4ノードすべてを使うので、1つでもデータなしなら海面(0m)とする。
            if [v00, v10, v01, v11].contains(&NO_DATA) {
                return 0.0;
            }
            // 東西方向に補間してから、南北方向に補間する。
            let (h00, h10, h01, h11) = (v00 as f32, v10 as f32, v01 as f32, v11 as f32);
            let h0 = h00 + (h10 - h00) * tx;
            let h1 = h01 + (h11 - h01) * tx;
            h0 + (h1 - h0) * ty
        };

        if level > 0 {
            // 表示中のレベルのグリッドが(解放などで)手元に無ければ、下のレベル0へ落ちる。
            let detail = tile.detail.borrow();
            if let Some(Some(cached)) = self.slot(level, chunk).and_then(|slot| detail.get(slot)) {
                let k = self.chunks_per_tile();
                let n_level = self.level_cells(level) as f64;
                let cells = self.chunk_cells(level);
                // タイル全体でのセル単位の座標(u*n_level)から、チャンクの南西角のセル位置を引いて、
                // チャンクのグリッド内の座標にする。
                return bilinear(
                    &cached.data,
                    cells,
                    u * n_level - (chunk % k * cells) as f64,
                    v * n_level - (chunk / k * cells) as f64,
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
        // 上限内なら候補集めの走査もしない(LODの更新のたびに呼ばれるので、普段はここで終わる)。
        if self.cached_bytes.get() <= limit_bytes {
            return;
        }
        let chunks = self.chunk_count();
        let mut candidates: Vec<(u64, usize, usize)> = Vec::new(); // (last_used, タイルidx, スロット)
        for (ti, tile) in self.tiles.iter().enumerate() {
            for (si, slot) in tile.detail.borrow().iter().enumerate() {
                if let Some(g) = slot {
                    // スロット番号 = (レベル-1)*チャンク数+チャンク番号 を逆算する。
                    let (level, chunk) = (si / chunks + 1, si % chunks);
                    if !keep(tile.key, chunk, level) {
                        candidates.push((g.last_used.get(), ti, si));
                    }
                }
            }
        }
        // `last_used`の小さい(=長く使われていない)順に並べ、上限を下回るまで先頭から捨てる。
        candidates.sort_unstable();
        for (_, ti, si) in candidates {
            if self.cached_bytes.get() <= limit_bytes {
                break;
            }
            let mut detail = self.tiles[ti].detail.borrow_mut();
            if let Some(g) = detail[si].take() {
                self.cached_bytes
                    .set(self.cached_bytes.get().saturating_sub(g.bytes()));
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
            default_origin: Origin {
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
    fn surface_matches_mesh_diagonal_and_ignores_unused_no_data_corner() {
        let data = one_tile(|lat, lon| if lat == 30.0 && lon == 120.0 { 1000 } else { 0 });
        let tile = data.tile(KEY).unwrap();
        let cell = 1.0 / data.level_cells(0) as f64;
        assert!((data.sample_surface(tile, 0.25 * cell, 0.25 * cell) - 500.0).abs() < 0.01);
        assert_eq!(data.sample_surface(tile, 0.75 * cell, 0.75 * cell), 0.0);
        assert_eq!(data.sample_surface(tile, 0.5 * cell, 0.5 * cell), 0.0);

        let coast = one_tile(|lat, lon| {
            if lat > 30.0 && lon > 120.0 {
                NO_DATA
            } else {
                100
            }
        });
        let tile = coast.tile(KEY).unwrap();
        assert_eq!(coast.sample_surface(tile, 0.25 * cell, 0.25 * cell), 100.0);
        assert_eq!(coast.sample_surface(tile, 0.75 * cell, 0.75 * cell), 0.0);
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
