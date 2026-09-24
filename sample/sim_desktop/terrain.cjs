'use strict';

const fs = require('node:fs/promises');
const path = require('node:path');
const { validateTerrain } = require('./runtime.cjs');

function terrainDirectory({ isPackaged, resourcesPath, env = process.env }) {
  // 配布版は開発環境の変数や過去の設定に依存せず、同梱データを使う。
  if (isPackaged) return path.join(resourcesPath, 'terrain');
  return env.SIM3DVIEW_TERRAIN_DIR
    ? path.resolve(env.SIM3DVIEW_TERRAIN_DIR)
    : path.resolve(__dirname, '../sim_server/assets/terrain');
}

async function bundleTerrain(source, resourcesPath) {
  await validateTerrain(source);
  const destination = path.join(resourcesPath, 'terrain');
  await fs.cp(source, destination, { recursive: true, dereference: true, errorOnExist: true, force: false });
  await validateTerrain(destination);
  return destination;
}

module.exports = { terrainDirectory, bundleTerrain };
