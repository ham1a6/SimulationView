'use strict';

// 実行中のOS・アーキテクチャ用の配布フォルダーを作る。
const fs = require('node:fs/promises');
const path = require('node:path');
const { platformLayout, developmentServer } = require('./platform.cjs');
const { terrainDirectory, bundleTerrain } = require('./terrain.cjs');
const { validateTerrain } = require('./runtime.cjs');

async function main() {
  const layout = platformLayout();
  if (process.arch !== 'x64') throw new Error('x64環境で実行してください。');
  const sampleRoot = path.resolve(__dirname, '..');
  const serverExe = path.resolve(process.env.SIM3DVIEW_SERVER_EXE || developmentServer('Release'));
  const terrain = terrainDirectory({ isPackaged: false });
  await validateTerrain(terrain);
  const frontend = path.join(__dirname, 'out/frontend');
  // 入力の不足はコピー開始前に検出する。
  await fs.access(serverExe);
  await fs.access(path.join(frontend, 'index.html'));
  const electronDist = path.dirname(require('electron'));
  const output = path.join(__dirname, 'out', `Sim3dView-${process.platform}-${process.arch}-${Date.now()}`);
  await fs.cp(electronDist, output, { recursive: true });
  await fs.rename(path.join(output, layout.electron), path.join(output, layout.app));
  const appDir = path.join(output, 'resources/app');
  const serverDir = path.join(output, 'resources/server');
  await fs.mkdir(appDir, { recursive: true });
  await fs.mkdir(serverDir, { recursive: true });
  for (const name of ['main.cjs', 'runtime.cjs', 'platform.cjs', 'terrain.cjs', 'package.json']) {
    await fs.copyFile(path.join(__dirname, name), path.join(appDir, name));
  }
  await fs.cp(frontend, path.join(appDir, 'frontend'), { recursive: true });
  await bundleTerrain(terrain, path.join(output, 'resources'));
  await fs.copyFile(serverExe, path.join(serverDir, layout.server));
  if (process.platform === 'linux') {
    await fs.chmod(path.join(output, layout.app), 0o755);
    await fs.chmod(path.join(serverDir, layout.server), 0o755);
  }
  for (const entry of await fs.readdir(path.dirname(serverExe))) {
    if (process.platform === 'win32' && entry.toLowerCase().endsWith('.dll')) await fs.copyFile(path.join(path.dirname(serverExe), entry), path.join(serverDir, entry));
  }
  await fs.copyFile(path.join(sampleRoot, 'THIRD_PARTY_NOTICE.md'), path.join(output, 'THIRD_PARTY_NOTICE.md'));
  // ElectronのLICENSEとLICENSES.chromium.htmlはdistのコピーで保持される。
  await fs.copyFile(path.join(__dirname, 'README.md'), path.join(output, 'README.md'));
  console.log(`配布フォルダー: ${output}\n地形データは resources/terrain に同梱しています。`);
}

main().catch(error => { console.error(error); process.exitCode = 1; });
