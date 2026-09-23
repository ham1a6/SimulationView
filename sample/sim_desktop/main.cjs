'use strict';

const { app, BrowserWindow, dialog, Menu } = require('electron');
const fs = require('node:fs/promises');
const path = require('node:path');
const { DesktopRuntime, validateTerrain } = require('./runtime.cjs');
app.setName('Sim3dView');

let runtime;
let window;
let quitting = false;
let stopped = false;
let failed = false;
const settingsPath = () => path.join(app.getPath('userData'), 'desktop-settings.json');

async function chooseTerrain() {
  const result = await dialog.showOpenDialog({
    title: '前処理済み地形フォルダーを選択', properties: ['openDirectory'],
    message: 'metadata.json、tile_index.json、base.bin、tilesが入ったフォルダーを選択してください。',
  });
  if (result.canceled) return null;
  const directory = result.filePaths[0];
  await validateTerrain(directory);
  await fs.mkdir(app.getPath('userData'), { recursive: true });
  await fs.writeFile(settingsPath(), JSON.stringify({ terrainDir: directory }, null, 2));
  return directory;
}

async function terrainDirectory() {
  if (process.env.SIM3DVIEW_TERRAIN_DIR) return path.resolve(process.env.SIM3DVIEW_TERRAIN_DIR);
  let saved;
  try { saved = JSON.parse(await fs.readFile(settingsPath(), 'utf8')).terrainDir; }
  catch (error) { if (error.code !== 'ENOENT' && !(error instanceof SyntaxError)) throw error; }
  const candidate = saved || (app.isPackaged
    ? path.join(path.dirname(process.execPath), 'terrain')
    : path.resolve(__dirname, '../sim_server/assets/terrain'));
  try { await validateTerrain(candidate); return candidate; }
  catch { return chooseTerrain(); }
}

function fail(error) {
  if (quitting || failed) return;
  failed = true;
  dialog.showErrorBox('Sim3dViewを起動・継続できません',
    `${error.message}\n\nUIのビルド、sim_server.exeと必要なDLL、地形フォルダーを確認してください。`);
  app.quit();
}

async function start() {
  const terrainDir = await terrainDirectory();
  if (!terrainDir || quitting) { app.quit(); return; }
  const serverExe = process.env.SIM3DVIEW_SERVER_EXE || (app.isPackaged
    ? path.join(process.resourcesPath, 'server/sim_server.exe')
    : path.resolve(__dirname, '../sim_server/build/Debug/sim_server.exe'));
  runtime = new DesktopRuntime({
    serverExe: path.resolve(serverExe), terrainDir,
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
      { label: '地形フォルダーを変更…', enabled: !process.env.SIM3DVIEW_TERRAIN_DIR, click: async () => {
        try { if (await chooseTerrain()) { app.relaunch(); app.quit(); } }
        catch (error) { dialog.showErrorBox('地形フォルダーを変更できません', error.message); }
      } },
      { type: 'separator' }, { label: '終了', click: () => app.quit() },
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
