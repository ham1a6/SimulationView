//! Pointer Eventsを使うドラッグ操作の純粋な状態管理。
//!
//! DOMのpointer captureや、移動量を何へ反映するかは呼び出し側が担当する。

/// 1回のpointer moveで得られる移動量。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DragUpdate {
    /// 直前の位置からの差分。
    pub delta: (f64, f64),
    /// ドラッグ開始位置からの差分。
    pub total: (f64, f64),
}

/// pointer upで確定したドラッグの情報。
#[derive(Debug, Clone, Copy, PartialEq)]
pub struct DragEnd {
    /// ドラッグ開始位置から、離した位置までの差分。
    pub total: (f64, f64),
}

impl DragEnd {
    /// 開始位置からの直線距離。
    pub fn distance(self) -> f64 {
        self.total.0.hypot(self.total.1)
    }
}

/// 追跡中のドラッグ。
#[derive(Debug, Clone, Copy, PartialEq)]
struct ActiveDrag {
    /// 追跡しているpointer(`PointerEvent::pointer_id`)。
    pointer_id: i32,
    /// 押した位置。
    start: (f64, f64),
    /// 最後に受け取った位置。
    current: (f64, f64),
}

/// 同時に1本のpointerだけを追跡するドラッグ状態。
#[derive(Debug, Clone, Copy, Default, PartialEq)]
pub struct DragTracker {
    /// 追跡中のドラッグ(押されていなければNone)。
    active: Option<ActiveDrag>,
}

impl DragTracker {
    /// pointer downの位置`(x, y)`からドラッグを始める(追跡中のものがあれば置き換える)。
    pub fn begin(&mut self, pointer_id: i32, x: f64, y: f64) {
        self.active = Some(ActiveDrag {
            pointer_id,
            start: (x, y),
            current: (x, y),
        });
    }

    /// pointer moveを反映して移動量を返す。追跡中のpointerでなければNone(何もしない)。
    pub fn update(&mut self, pointer_id: i32, x: f64, y: f64) -> Option<DragUpdate> {
        let active = self
            .active
            .as_mut()
            .filter(|a| a.pointer_id == pointer_id)?;
        let previous = active.current;
        active.current = (x, y);
        Some(DragUpdate {
            delta: (x - previous.0, y - previous.1),
            total: (x - active.start.0, y - active.start.1),
        })
    }

    /// pointer upでドラッグを終え、開始位置からの移動量を返す。追跡中のpointerでなければNone。
    pub fn end(&mut self, pointer_id: i32, x: f64, y: f64) -> Option<DragEnd> {
        let active = self.active.filter(|a| a.pointer_id == pointer_id)?;
        self.active = None;
        Some(DragEnd {
            total: (x - active.start.0, y - active.start.1),
        })
    }

    /// pointer cancelでドラッグを取り消す。追跡中のpointerだったらtrue。
    pub fn cancel(&mut self, pointer_id: i32) -> bool {
        if self.active.is_some_and(|a| a.pointer_id == pointer_id) {
            self.active = None;
            true
        } else {
            false
        }
    }

    /// ドラッグを追跡中か。
    pub fn is_active(self) -> bool {
        self.active.is_some()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn reports_incremental_and_total_delta() {
        let mut drag = DragTracker::default();
        drag.begin(7, 10.0, 20.0);
        assert_eq!(
            drag.update(7, 13.0, 18.0),
            Some(DragUpdate {
                delta: (3.0, -2.0),
                total: (3.0, -2.0),
            })
        );
        assert_eq!(drag.update(7, 15.0, 25.0).unwrap().delta, (2.0, 7.0));
        assert_eq!(drag.end(7, 16.0, 28.0).unwrap().distance(), 10.0);
        assert!(!drag.is_active());
    }

    #[test]
    fn ignores_another_pointer_and_matching_cancel_ends_drag() {
        let mut drag = DragTracker::default();
        drag.begin(1, 0.0, 0.0);
        assert_eq!(drag.update(2, 5.0, 5.0), None);
        assert!(!drag.cancel(2));
        assert!(drag.is_active());
        assert!(drag.cancel(1));
        assert!(!drag.is_active());
    }
}
