'use strict';

// Electron公式の手動配布方式。別のパッケージャーを追加せずWindows用フォルダーを作る。
const fs = require('node:fs/promises');
const path = require('node:path');

async function main() {
  if (process.platform !== 'win32' || process.arch !== 'x64') throw new Error('Windows x64で実行してください。');
  const repo = path.resolve(__dirname, '../..');
  const serverExe = path.resolve(process.env.SIM3DVIEW_SERVER_EXE || path.join(repo, 'sample/sim_server/build/Release/sim_server.exe'));
  const frontend = path.join(__dirname, 'out/frontend');
  // 入力の不足はコピー開始前に検出する。
  await fs.access(serverExe);
  await fs.access(path.join(frontend, 'index.html'));
  const electronDist = path.dirname(require('electron'));
  const output = path.join(__dirname, 'out', `Sim3dView-win32-x64-${Date.now()}`);
  await fs.cp(electronDist, output, { recursive: true });
  await fs.rename(path.join(output, 'electron.exe'), path.join(output, 'Sim3dView.exe'));
  const appDir = path.join(output, 'resources/app');
  const serverDir = path.join(output, 'resources/server');
  await fs.mkdir(appDir, { recursive: true });
  await fs.mkdir(serverDir, { recursive: true });
  for (const name of ['main.cjs', 'runtime.cjs', 'package.json']) {
    await fs.copyFile(path.join(__dirname, name), path.join(appDir, name));
  }
  await fs.cp(frontend, path.join(appDir, 'frontend'), { recursive: true });
  await fs.copyFile(serverExe, path.join(serverDir, 'sim_server.exe'));
  for (const entry of await fs.readdir(path.dirname(serverExe))) {
    if (entry.toLowerCase().endsWith('.dll')) await fs.copyFile(path.join(path.dirname(serverExe), entry), path.join(serverDir, entry));
  }
  await fs.copyFile(path.join(repo, 'THIRD_PARTY_NOTICE.md'), path.join(output, 'THIRD_PARTY_NOTICE.md'));
  // ElectronのLICENSEとLICENSES.chromium.htmlはdistのコピーで保持される。
  await fs.copyFile(path.join(__dirname, 'README.md'), path.join(output, 'README.md'));
  console.log(`配布フォルダー: ${output}\n地形は同梱していません。初回起動時に前処理済みフォルダーを選択してください。`);
}

main().catch(error => { console.error(error); process.exitCode = 1; });
