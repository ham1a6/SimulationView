//! 地形メッシュの解像度レベル切り替えのクロスフェード(状態と割合の計算。GPUには触れない)。
//!
//! チャンクのレベルが変わる・タイル全体とチャンクが入れ替わるとき、古いメッシュを消して新しい
//! メッシュを出すだけだと、細かさ・陰影の違いが一瞬で切り替わって目立つ。そこで、古いメッシュを
//! `FADE_DURATION_MS`だけ残し、新旧を画素ごとの疎密(ディザ)で互いに補う割合で重ねて描く
//! (`terrain.wgsl`の`fs_fade`)。割合は時間の経過から`fade_progress`で求める。

use std::collections::HashMap;

use crate::terrain::loader::MeshKey;

/// クロスフェードにかける時間(ミリ秒)。
pub(super) const FADE_DURATION_MS: f64 = 350.0;
/// 同時にクロスフェードさせるメッシュ(新旧を別々に数える)の上限。`terrain.wgsl`の`FadeTable`の要素数に合わせる。
/// 超える分は、クロスフェードせずすぐに切り替える。
pub(super) const MAX_FADE_ENTRIES: usize = 2048;

/// 開始(`start_ms`)から`now_ms`までの、新しいメッシュを表示する画素の割合(0〜1)。なめらかに始まり、なめらかに終わる。
pub(super) fn fade_progress(start_ms: f64, now_ms: f64) -> f32 {
    let t = ((now_ms - start_ms) / FADE_DURATION_MS).clamp(0.0, 1.0) as f32;
    t * t * (3.0 - 2.0 * t)
}

/// 消えていく途中の古いメッシュ。
pub(super) struct Outgoing<T> {
    pub key: MeshKey,
    pub mesh: T,
    pub start_ms: f64,
}

/// クロスフェード中のメッシュの一覧。`T`は古いメッシュの実体(`MeshGpu`)。
pub(super) struct Fades<T> {
    /// 出てくる途中の新しいメッシュ(`TerrainRenderer::meshes`にある)と、その開始時刻。
    incoming: HashMap<MeshKey, f64>,
    outgoing: Vec<Outgoing<T>>,
}

impl<T> Fades<T> {
    pub(super) fn new() -> Self {
        Self {
            incoming: HashMap::new(),
            outgoing: Vec::new(),
        }
    }

    /// 進行中のクロスフェードがあるか(あれば、時間が進むので描き直し続ける必要がある)。
    pub(super) fn is_active(&self) -> bool {
        !self.incoming.is_empty() || !self.outgoing.is_empty()
    }

    /// 新しいクロスフェードを始める余地があるか。
    pub(super) fn has_room(&self) -> bool {
        self.incoming.len() + self.outgoing.len() < MAX_FADE_ENTRIES
    }

    /// `key`のメッシュを新しいものへ差し替えたときに呼ぶ。`old`は差し替え前のメッシュ(無ければ`None`)。
    /// 新しいメッシュを出す側に、`old`を消える側に登録する。前のクロスフェードの途中だったら、その古い側は
    /// 捨てる(いま出ている側が改めて消える側になる)。
    pub(super) fn replace(&mut self, key: MeshKey, old: Option<T>, now_ms: f64) {
        self.cancel(key);
        self.incoming.insert(key, now_ms);
        if let Some(mesh) = old {
            self.push_outgoing(key, mesh, now_ms);
        }
    }

    /// `key`のメッシュ`old`を取り除いたときに呼ぶ(消える側に登録する)。
    pub(super) fn remove(&mut self, key: MeshKey, old: T, now_ms: f64) {
        self.cancel(key);
        self.push_outgoing(key, old, now_ms);
    }

    fn push_outgoing(&mut self, key: MeshKey, mesh: T, start_ms: f64) {
        self.outgoing.push(Outgoing {
            key,
            mesh,
            start_ms,
        });
    }

    /// `key`のクロスフェードを、すぐに終わらせる(そのメッシュがすぐに置き換わる・頂点を書き換えるとき)。
    pub(super) fn cancel(&mut self, key: MeshKey) {
        self.outgoing.retain(|o| o.key != key);
        self.incoming.remove(&key);
    }

    /// すべてのクロスフェードをすぐに終わらせる。
    pub(super) fn clear(&mut self) {
        self.outgoing.clear();
        self.incoming.clear();
    }

    /// 終わったクロスフェードを片付ける(古いメッシュはここで解放される)。
    pub(super) fn finish(&mut self, now_ms: f64) {
        self.outgoing
            .retain(|o| now_ms - o.start_ms < FADE_DURATION_MS);
        self.incoming
            .retain(|_, &mut start| now_ms - start < FADE_DURATION_MS);
    }

    /// 出てくる途中のメッシュなら、その割合(0〜1)。そうでなければ`None`(=普通に描く)。
    pub(super) fn incoming_progress(&self, key: &MeshKey, now_ms: f64) -> Option<f32> {
        self.incoming
            .get(key)
            .map(|&start| fade_progress(start, now_ms))
    }

    /// 出てくる途中のメッシュか。
    pub(super) fn is_incoming(&self, key: &MeshKey) -> bool {
        self.incoming.contains_key(key)
    }

    /// 出てくる途中のメッシュがあるか(無ければ、描くときにメッシュごとの照会を省ける)。
    pub(super) fn has_incoming(&self) -> bool {
        !self.incoming.is_empty()
    }

    /// 消える途中の古いメッシュ。
    pub(super) fn outgoing(&self) -> &[Outgoing<T>] {
        &self.outgoing
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    const KEY: MeshKey = (35, 138, 3);

    #[test]
    fn progress_goes_from_zero_to_one_smoothly() {
        assert_eq!(fade_progress(100.0, 100.0), 0.0);
        assert_eq!(fade_progress(100.0, 100.0 + FADE_DURATION_MS), 1.0);
        assert!((fade_progress(100.0, 100.0 + FADE_DURATION_MS / 2.0) - 0.5).abs() < 1e-6);
        // 開始前・終了後は端の値のまま。
        assert_eq!(fade_progress(100.0, 0.0), 0.0);
        assert_eq!(fade_progress(100.0, 1e9), 1.0);
        // 単調増加。
        let mut last = 0.0;
        for i in 0..=35 {
            let p = fade_progress(0.0, i as f64 * 10.0);
            assert!(p >= last);
            last = p;
        }
    }

    #[test]
    fn replacing_keeps_the_old_mesh_until_the_fade_ends() {
        let mut fades: Fades<&str> = Fades::new();
        assert!(!fades.is_active());
        fades.replace(KEY, Some("old"), 1000.0);
        assert!(fades.is_active());
        assert_eq!(fades.outgoing().len(), 1);
        assert_eq!(fades.incoming_progress(&KEY, 1000.0), Some(0.0));

        fades.finish(1000.0 + FADE_DURATION_MS - 1.0);
        assert!(fades.is_active());
        assert_eq!(fades.outgoing().len(), 1);

        fades.finish(1000.0 + FADE_DURATION_MS);
        assert!(!fades.is_active());
        assert!(fades.outgoing().is_empty());
        assert_eq!(fades.incoming_progress(&KEY, 2000.0), None);
    }

    #[test]
    fn a_new_mesh_without_an_old_one_only_fades_in() {
        let mut fades: Fades<&str> = Fades::new();
        fades.replace(KEY, None, 0.0);
        assert!(fades.has_incoming());
        assert!(fades.outgoing().is_empty());
    }

    #[test]
    fn replacing_again_during_a_fade_drops_the_older_mesh() {
        let mut fades: Fades<&str> = Fades::new();
        fades.replace(KEY, Some("level1"), 0.0);
        // level2が出ている途中で、level3へ差し替える: level1は捨て、level2が消える側になる。
        fades.replace(KEY, Some("level2"), 100.0);
        let names: Vec<&str> = fades.outgoing().iter().map(|o| o.mesh).collect();
        assert_eq!(names, ["level2"]);
        assert_eq!(fades.outgoing()[0].start_ms, 100.0);
        assert_eq!(fades.incoming_progress(&KEY, 100.0), Some(0.0));
    }

    #[test]
    fn removing_fades_the_mesh_out_and_cancels_its_fade_in() {
        let mut fades: Fades<&str> = Fades::new();
        fades.replace(KEY, None, 0.0);
        fades.remove(KEY, "gone", 50.0);
        assert!(!fades.has_incoming());
        assert_eq!(fades.outgoing().len(), 1);
        fades.finish(50.0 + FADE_DURATION_MS);
        assert!(!fades.is_active());
    }

    #[test]
    fn cancel_and_clear_end_fades_immediately() {
        let mut fades: Fades<&str> = Fades::new();
        fades.replace(KEY, Some("a"), 0.0);
        fades.replace((1, 2, 3), Some("b"), 0.0);
        fades.cancel(KEY);
        assert_eq!(fades.incoming_progress(&KEY, 0.0), None);
        assert_eq!(fades.outgoing().len(), 1);
        fades.clear();
        assert!(!fades.is_active());
    }

    #[test]
    fn room_runs_out_at_the_table_size() {
        let mut fades: Fades<u32> = Fades::new();
        for i in 0..MAX_FADE_ENTRIES as i32 {
            assert!(fades.has_room());
            fades.replace((i, 0, 0), None, 0.0);
        }
        assert!(!fades.has_room());
    }
}
