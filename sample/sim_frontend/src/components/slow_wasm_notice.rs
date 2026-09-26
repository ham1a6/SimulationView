//! WebAssemblyがJITなしで動いている(ブラウザがJITを止めている)ことを検知して、画面上部に
//! 案内を出す帯(DETAILED_DESIGN.md 7.11節)。
//!
//! Microsoft Edgeの「Webのセキュリティを強化する」は、訪問の少ないサイト(localhost・IPアドレスの
//! URLを含む)でJITを止め、WebAssemblyをインタプリタで実行する。地形メッシュ生成や覆域計算が
//! 数十倍遅くなるが、ページ側からは止められないので、検知して利用者に例外登録を案内する。
//!
//! 検知: 手書きの極小WebAssemblyモジュール(整数ループ)の実行時間を測る。アプリ本体のwasmで
//! 測らないのは、開発ビルド(`sim_frontend`は最適化なし)とリリースビルドで速さが変わるため。
//! JITありは1回あたり約0.3〜0.6ns、インタプリタはループ1回に16命令を逐次解釈するので桁違いに遅い。
//! その間の`SLOW_NS_PER_ITER`をしきい値にする。GCなどの割り込みで誤判定しないよう、
//! しきい値を下回る回が1度でもあれば正常とみなす(最大`MAX_RUNS`回)。

use leptos::prelude::*;
use wasm_bindgen::{JsCast, JsValue};
use wasm_bindgen_futures::JsFuture;

/// 1回の計測のループ回数。JITありなら1ms前後で終わる。
const ITERATIONS: u32 = 2_000_000;
/// これより遅ければ(1回あたりのns)JITなしとみなす。JITありの約5〜10倍、インタプリタより十分小さい値。
const SLOW_NS_PER_ITER: f64 = 3.0;
/// 計測の最大回数。速い回が出たらそこで打ち切る。
const MAX_RUNS: usize = 3;

/// `(func (export "f") (param i32) (result i32))`: `s += i * i`を`i`が引数に達するまで繰り返し、`s`を返す。
const PROBE_WASM: [u8; 62] = [
    0x00, 0x61, 0x73, 0x6d, 0x01, 0x00, 0x00, 0x00, // マジック・バージョン
    0x01, 0x06, 0x01, 0x60, 0x01, 0x7f, 0x01, 0x7f, // 型: (i32) -> i32
    0x03, 0x02, 0x01, 0x00, // 関数: 型0
    0x07, 0x05, 0x01, 0x01, 0x66, 0x00, 0x00, // エクスポート: "f" = 関数0
    0x0a, 0x21, 0x01, 0x1f, 0x01, 0x02, 0x7f, // コード: ローカルi32×2(i, s)
    0x03, 0x40, // loop
    0x20, 0x02, 0x20, 0x01, 0x20, 0x01, 0x6c, 0x6a, 0x21, 0x02, // s = s + i * i
    0x20, 0x01, 0x41, 0x01, 0x6a, 0x22, 0x01, // i = i + 1
    0x20, 0x00, 0x48, 0x0d, 0x00, // i < n なら先頭へ
    0x0b, 0x20, 0x02, 0x0b, // end、sを返す
];

/// 検知モジュールで1回あたりの実行時間(ns)を測り、最速の値を返す。計測できなければ`None`。
async fn measure_ns_per_iter() -> Option<f64> {
    let performance = web_sys::window()?.performance()?;
    let promise = js_sys::WebAssembly::instantiate_buffer(&PROBE_WASM, &js_sys::Object::new());
    let result = JsFuture::from(promise).await.ok()?;
    let instance = js_sys::Reflect::get(&result, &"instance".into()).ok()?;
    let exports = js_sys::Reflect::get(&instance, &"exports".into()).ok()?;
    let f: js_sys::Function = js_sys::Reflect::get(&exports, &"f".into()).ok()?.dyn_into().ok()?;
    let n = JsValue::from(ITERATIONS);
    let mut best = f64::INFINITY;
    for _ in 0..MAX_RUNS {
        let start = performance.now();
        f.call1(&JsValue::NULL, &n).ok()?;
        let ns = (performance.now() - start) * 1.0e6 / f64::from(ITERATIONS);
        best = best.min(ns);
        if best < SLOW_NS_PER_ITER {
            break;
        }
    }
    Some(best)
}

fn is_edge() -> bool {
    web_sys::window()
        .and_then(|w| w.navigator().user_agent().ok())
        .is_some_and(|ua| ua.contains("Edg/"))
}

/// 起動時に1度だけ計測し、JITなしと判定したら閉じられる案内の帯を出す。ログパネルにも警告を残す。
#[component]
pub fn SlowWasmNotice() -> impl IntoView {
    let slow = RwSignal::new(false);
    let edge = is_edge();
    wasm_bindgen_futures::spawn_local(async move {
        let Some(ns) = measure_ns_per_iter().await else {
            return;
        };
        if ns >= SLOW_NS_PER_ITER {
            log::warn!(
                "WebAssemblyがJITなしで動いているため、地図・覆域の更新が遅くなります(計測 {ns:.1}ns/回)"
            );
            slow.set(true);
        }
    });
    let message = if edge {
        "Edgeの「Webのセキュリティを強化する」がこのサイトで有効なため、地図・覆域の更新が遅くなっています。\
         アドレスバー左のアイコン →「このサイトのセキュリティ強化」をオフにするか、\
         設定 → プライバシー、検索、サービス → セキュリティ の「例外」にこのサイトを追加してください。"
    } else {
        "ブラウザがこのサイトでJIT(WebAssemblyの最適化)を止めているため、地図・覆域の更新が遅くなっています。\
         ブラウザのセキュリティ強化機能の例外にこのサイトを追加してください。"
    };
    view! {
        <Show when=move || slow.get()>
            <div class="slow-wasm-notice" role="alert">
                <span class="slow-wasm-notice-text">{message}</span>
                <button class="slow-wasm-notice-close" title="閉じる" on:click=move |_| slow.set(false)>
                    "×"
                </button>
            </div>
        </Show>
    }
}
