'use strict';

const { app, BrowserWindow, dialog, Menu } = require('electron');
const path = require('node:path');
const { DesktopRuntime } = require('./runtime.cjs');
const { terrainDirectory } = require('./terrain.cjs');
const { platformLayout, developmentServer } = require('./platform.cjs');
app.setName('Sim3dView');

let runtime;
let window;
let quitting = false;
let stopped = false;
let failed = false;
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
  runtime = new DesktopRuntime({
    serverExe: path.resolve(serverExe), terrainDir,
    certFile: process.env.SIM3DVIEW_CERT_FILE || (app.isPackaged ? path.join(process.resourcesPath, 'certs', 'dev-cert.pem') : path.join(__dirname, '../certs/dev-cert.pem')),
    keyFile: process.env.SIM3DVIEW_KEY_FILE || (app.isPackaged ? path.join(process.resourcesPath, 'certs', 'dev-key.pem') : path.join(__dirname, '../certs/dev-key.pem')),
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
