'use strict';

const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const os = require('node:os');
const path = require('node:path');
const { terrainDirectory, bundleTerrain } = require('../terrain.cjs');

test('配布版は環境変数より同梱地形を優先する', () => {
  const resourcesPath = path.resolve('配布先/resources');
  const env = { SIM3DVIEW_TERRAIN_DIR: path.resolve('外部地形') };
  assert.equal(terrainDirectory({ isPackaged: true, resourcesPath, env }), path.join(resourcesPath, 'terrain'));
  assert.equal(terrainDirectory({ isPackaged: false, env }), env.SIM3DVIEW_TERRAIN_DIR);
});

test('地形の全階層をコピーし、コピー元なしで配布データを読める', async t => {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), 'sim3dview-bundle-'));
  t.after(() => fs.rm(root, { recursive: true, force: true }));
  const source = path.join(root, '入力地形');
  await fs.mkdir(path.join(source, 'tiles/L4'), { recursive: true });
  const files = { 'metadata.json': '{}', 'tile_index.json': '[]', 'base.bin': 'base', 'tiles/L4/N035E138.bin': '詳細地形' };
  for (const [name, data] of Object.entries(files)) await fs.writeFile(path.join(source, name), data);
  const destination = await bundleTerrain(source, path.join(root, 'resources'));
  await fs.rename(source, path.join(root, '移動済み'));
  for (const [name, data] of Object.entries(files)) assert.equal(await fs.readFile(path.join(destination, name), 'utf8'), data);
  await assert.rejects(bundleTerrain(path.join(root, '不存在'), path.join(root, '欠損配布')));
  await assert.rejects(fs.stat(path.join(root, '欠損配布')));
});
