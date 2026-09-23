'use strict';

// 実際のメインプロセスを起動し、WASM・GPU・再読み込みを検証する。
const { app, BrowserWindow, dialog } = require('electron');
const fs = require('node:fs/promises');
const path = require('node:path');
const assert = require('node:assert/strict');
const output = process.env.SIM3DVIEW_TEST_OUTPUT || path.resolve(__dirname, '../out/smoke');
app.setPath('userData', path.join(output, `profile-${process.pid}`));
process.env.SIM3DVIEW_TERRAIN_DIR ||= path.resolve(__dirname, '../../sim_server/assets/terrain');
const errors = [];
// オフライン検証ではOS設定を変更せず、テスト用セッションの外部通信を拒否する。
const offline = process.env.SIM3DVIEW_TEST_OFFLINE === '1';
const externalRequests = [];
const localRequests = new Set();
let passed = false;
// 自動テストではネイティブのエラーダイアログで待ち続けず失敗を記録する。
dialog.showErrorBox = (title, message) => { errors.push(`${title}: ${message}`); console.error(title, message); };
const sleep = ms => new Promise(resolve => setTimeout(resolve, ms));
async function until(check) {
  for (let count = 0; count < 120; count++) {
    if (await check()) return;
    await sleep(500);
  }
  throw new Error('画面の準備がタイムアウトしました。');
}
let backendPort;
let frontendOrigin;
app.on('browser-window-created', (_event, window) => {
  if (offline) {
    window.webContents.session.webRequest.onBeforeRequest((details, callback) => {
      const url = new URL(details.url);
      const network = ['http:', 'https:', 'ws:', 'wss:'].includes(url.protocol);
      const external = network && !['127.0.0.1', 'localhost', '[::1]'].includes(url.hostname);
      if (external) externalRequests.push(details.url);
      else if (network) localRequests.add(details.url);
      callback({ cancel: external });
    });
  }
  window.webContents.on('console-message', details => {
    if (details.level === 'error') errors.push(details.message);
  });
});
require('../main.cjs');

app.whenReady().then(async () => {
  await fs.mkdir(output, { recursive: true });
  await until(() => BrowserWindow.getAllWindows().length > 0);
  const window = BrowserWindow.getAllWindows()[0];
  for (let pass = 0; pass < 2; pass++) {
    window.show();
    window.focus();
    await until(async () => {
      if (window.webContents.isLoading()) return false;
      return window.webContents.executeJavaScript(`document.querySelector('canvas') !== null && !document.body.innerText.includes('接続中')`);
    });
    await sleep(3000);
    const click = label => window.webContents.executeJavaScript(`(() => {
      const button = [...document.querySelectorAll('button')].find(b => b.innerText === ${JSON.stringify(label)});
      if (!button) throw new Error('ボタンがありません');
      button.click();
    })()`);
    await click('開始');
    await until(() => window.webContents.executeJavaScript(`document.body.innerText.includes('シミュレーション実行中')`));
    await sleep(300);
    await click('一時停止');
    await until(() => window.webContents.executeJavaScript(`document.body.innerText.includes('一時停止中')`));
    await click('2D表示に切替');
    await until(() => window.webContents.executeJavaScript(`document.body.innerText.includes('3D表示に切替')`));
    await click('3D表示に切替');
    // 非表示・遮蔽中はResizeObserverが遅延するため、実寸への追従も確認する。
    await until(() => window.webContents.executeJavaScript(`(() => {
      const canvas = document.querySelector('canvas');
      const rect = canvas.getBoundingClientRect();
      return Math.abs(canvas.width - rect.width) < 2 && Math.abs(canvas.height - rect.height) < 2;
    })()`));
    const info = await window.webContents.executeJavaScript(`({
      url: location.href, text: document.body.innerText,
      buttons: [...document.querySelectorAll('button')].map(b => b.innerText),
      canvas: [...document.querySelectorAll('canvas')].map(c => [c.width, c.height]),
      gpu: !!navigator.gpu, node: typeof require,
    })`);
    console.log(JSON.stringify({ url: info.url, canvas: info.canvas, gpu: info.gpu, node: info.node, pass }));
    assert.equal(info.gpu, true);
    assert.equal(info.node, 'undefined');
    assert.ok(info.canvas.some(([width, height]) => width > 0 && height > 0));
    backendPort = new URL(info.url).searchParams.get('sim_port');
    frontendOrigin = new URL(info.url).origin;
    await fs.writeFile(path.join(output, `screen-${pass}.png`), (await window.webContents.capturePage()).toPNG());
    if (pass === 0) {
      window.webContents.reload();
      await sleep(1000);
    }
  }
  assert.deepEqual(errors, [], `描画コンソールのエラー: ${errors.join('\n')}`);
  if (offline) {
    assert.deepEqual(externalRequests, [], '外部通信への依存があります。');
    assert.ok([...localRequests].some(url => url.endsWith('.wasm')), 'WASMの取得を確認できません。');
    assert.ok([...localRequests].some(url => url.includes('/terrain/base.bin')), '地形の取得を確認できません。');
    assert.ok([...localRequests].some(url => url.includes('/sim')), 'シミュレータ接続を確認できません。');
    await fs.writeFile(path.join(output, 'offline-requests.json'), JSON.stringify({ externalRequests, localRequests: [...localRequests] }, null, 2));
    console.log(`オフライン検証完了: 外部通信${externalRequests.length}件、ローカルURL ${localRequests.size}件`);
  }
  console.log(`描画確認完了: ${output}`);
  passed = true;
  window.close();
}).catch(error => {
  console.error(error);
  process.exitCode = 1;
  app.quit();
});

app.on('will-quit', event => {
  event.preventDefault();
  console.log(`終了対象ポート: UI=${frontendOrigin} backend=${backendPort}`);
  Promise.all([frontendOrigin, `http://127.0.0.1:${backendPort}/terrain/metadata.json`].map(async url => {
    if (!passed) return;
    await assert.rejects(fetch(url, { signal: AbortSignal.timeout(2000) }));
  })).then(() => app.exit(passed ? 0 : 1)).catch(error => { console.error(error); app.exit(1); });
});
