'use strict';

const { app, BrowserWindow, dialog, Menu } = require('electron');
const path = require('node:path');
const { readFile } = require('node:fs/promises');
const { X509Certificate } = require('node:crypto');
const { DesktopRuntime } = require('./runtime.cjs');
const { terrainDirectory } = require('./terrain.cjs');
const { platformLayout, developmentServer } = require('./platform.cjs');
app.setName('Sim3dView');

let runtime;
let window;
let quitting = false;
let stopped = false;
let failed = false;

async function certificateFingerprint256(file) {
  return new X509Certificate(await readFile(file)).fingerprint256;
}

function isLoopback(hostname) {
  return hostname === '127.0.0.1' || hostname === '::1' || hostname === 'localhost';
}
function fail(error) {
  if (quitting || failed) return;
  failed = true;
  dialog.showErrorBox('Sim3dViewを起動・継続できません',
    `${error.message}\n\nUIのビルド、sim_server実行ファイルと必要な共有ライブラリ、地形フォルダーを確認してください。`);
  app.quit();
}

async function start() {
  const terrainDir = terrainDirectory({ isPackaged: app.isPackaged, resourcesPath: process.resourcesPath });
  if (!terrainDir || quitting) { app.quit(); return; }
  const serverExe = process.env.SIM3DVIEW_SERVER_EXE || (app.isPackaged
    ? path.join(process.resourcesPath, 'server', platformLayout().server)
    : developmentServer());
  const certFile = process.env.SIM3DVIEW_CERT_FILE || (app.isPackaged ? path.join(process.resourcesPath, 'certs', 'dev-cert.pem') : path.join(__dirname, '../certs/dev-cert.pem'));
  const keyFile = process.env.SIM3DVIEW_KEY_FILE || (app.isPackaged ? path.join(process.resourcesPath, 'certs', 'dev-key.pem') : path.join(__dirname, '../certs/dev-key.pem'));
  const localCertificateFingerprint = await certificateFingerprint256(certFile);
  runtime = new DesktopRuntime({
    serverExe: path.resolve(serverExe), terrainDir,
    certFile, keyFile,
    frontendDir: app.isPackaged ? path.join(__dirname, 'frontend') : path.join(__dirname, 'out/frontend'),
    onFailure: fail,
  });
  const url = await runtime.start();
  if (quitting) return;
  window = new BrowserWindow({
    title: 'Sim3dView', width: 1440, height: 960, minWidth: 800, minHeight: 600,
    backgroundColor: '#151a22',
    webPreferences: { nodeIntegration: false, contextIsolation: true, sandbox: true },
  });
  // UIにNode/IPCを公開せず、外部ページや別ウィンドウへの遷移を禁止する。
  const origin = new URL(url).origin;
  window.webContents.on('will-navigate', (event, target) => {
    if (new URL(target).origin !== origin) event.preventDefault();
  });
  window.webContents.setWindowOpenHandler(() => ({ action: 'deny' }));
  window.webContents.session.setPermissionRequestHandler((_contents, _permission, callback) => callback(false));
  // 地形fetchもWebTransportと同じ自己署名証明書を使う。ループバックかつ同梱証明書と
  // SHA-256が一致するときだけ、Chromiumの通常検証による失敗を上書きして受理する。
  window.webContents.session.setCertificateVerifyProc((request, callback) => {
    try {
      const fingerprint = new X509Certificate(request.certificate.data).fingerprint256;
      callback(isLoopback(request.hostname) && fingerprint === localCertificateFingerprint ? 0 : -3);
    } catch {
      callback(-3);
    }
  });
  window.webContents.on('render-process-gone', (_event, details) => fail(new Error(`描画プロセスが終了しました: ${details.reason}`)));
  Menu.setApplicationMenu(Menu.buildFromTemplate([
    { label: 'ファイル', submenu: [
      { label: '終了', click: () => app.quit() },
    ] },
    { label: '表示', submenu: [
      { label: '再読み込み', role: 'reload' }, { label: '全画面表示', role: 'togglefullscreen' },
      { label: '開発者ツール', role: 'toggleDevTools' },
    ] },
  ]));
  await window.loadURL(url);
}

if (!app.requestSingleInstanceLock()) {
  app.quit();
} else {
  app.on('second-instance', () => { if (window) { if (window.isMinimized()) window.restore(); window.focus(); } });
  app.on('window-all-closed', () => app.quit());
  app.on('before-quit', event => {
    if (stopped) return;
    event.preventDefault();
    if (quitting) return;
    quitting = true;
    Promise.resolve(runtime?.stop()).finally(() => { stopped = true; app.quit(); });
  });
  app.whenReady().then(start).catch(fail);
}
