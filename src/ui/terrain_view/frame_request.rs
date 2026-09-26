//! 入力・通信・フェードからの再描画要求を、画面更新ごとにまとめる。

/// `requestAnimationFrame`の予約が1つだけになるようにするための印。
/// 予約したら立て、コールバックの実行が始まったら(または予約に失敗したら)下ろす。
#[derive(Default)]
pub(super) struct FrameRequest {
    /// 予約済みで、まだコールバックが走っていないか。
    pending: bool,
}

impl FrameRequest {
    /// 新しいコールバックの予約が必要なときだけtrueを返す。
    pub(super) fn request(&mut self) -> bool {
        !std::mem::replace(&mut self.pending, true)
    }

    /// コールバックの実行開始、または予約失敗時に解除する。
    pub(super) fn clear(&mut self) {
        self.pending = false;
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn input_and_fade_requests_share_one_callback_per_frame() {
        let mut request = FrameRequest::default();
        let mut queued = 0;
        let mut rendered = 0;
        let mut latest = 0;
        for frame in 0..60 {
            // 1回の画面更新までに入力・通信が合計10回到着する。
            for event in 1..=10 {
                latest = frame * 10 + event;
                queued += usize::from(request.request());
            }
            assert_eq!(queued, 1);
            queued -= 1;
            request.clear();
            rendered += 1;
            assert_eq!(latest, (frame + 1) * 10);
            // フェード中の自己予約と、次の入力による予約も重複させない。
            if frame < 59 {
                queued += usize::from(request.request());
            }
        }
        assert_eq!(rendered, 60);
        assert_eq!(queued, 0); // フェードも入力も止まれば描き続けない。
    }

    #[test]
    fn failed_reservation_does_not_block_later_updates() {
        let mut request = FrameRequest::default();
        assert!(request.request());
        request.clear(); // ブラウザーが予約に失敗した場合。
        assert!(request.request());
        assert!(!request.request());
    }
}
