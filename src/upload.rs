//! ブラウザが保持するファイルを、アプリ指定のURLへ送信する。
//! 保存先・認証・レスポンスの解釈は呼び出し側が担当する。

use gloo_net::http::Request;

/// Blob（Fileも可）をHTTP POSTの本文として送る。
///
/// multipartではなく生バイトを送信する。Content-Typeはapplication/octet-stream。
/// `headers`で認証などのアプリ固有ヘッダーを指定できる。
/// ブラウザのCORS・HTTPS制約に従い、非2xxはエラーとして返す。
/// ファイル全体をRustのメモリへコピーしない。
pub async fn upload_blob(
    url: &str,
    blob: &web_sys::Blob,
    headers: &[(&str, &str)],
) -> Result<String, String> {
    let mut request = Request::post(url).header("Content-Type", "application/octet-stream");
    for &(name, value) in headers {
        request = request.header(name, value);
    }
    let response = request
        .body(blob)
        .map_err(|e| format!("送信要求を作成できません: {e}"))?
        .send()
        .await
        .map_err(|e| format!("ファイルを送信できません: {e}"))?;
    if !response.ok() {
        return Err(format!(
            "ファイル送信に失敗しました: HTTP {}",
            response.status()
        ));
    }
    response
        .text()
        .await
        .map_err(|e| format!("送信結果を読み取れません: {e}"))
}
