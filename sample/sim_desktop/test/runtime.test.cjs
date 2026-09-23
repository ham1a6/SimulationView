'use strict';

const { test } = require('node:test');
const assert = require('node:assert/strict');
const fs = require('node:fs/promises');
const path = require('node:path');
const os = require('node:os');
const http = require('node:http');
const { DesktopRuntime, serveFrontend, validateTerrain } = require('../runtime.cjs');
const { developmentServer } = require('../platform.cjs');

test('UI配信はWASMのMIME、HEAD、外部パス・Host拒否を維持する', async t => {
  const root = await fs.mkdtemp(path.join(os.tmpdir(), 'sim3dview-'));
  t.after(() => fs.rm(root, { recursive: true, force: true }));
  const frontend = path.join(root, 'frontend');
  await fs.mkdir(frontend);
  await fs.writeFile(path.join(frontend, 'index.html'), '<title>test</title>');
  await fs.writeFile(path.join(frontend, 'test.wasm'), 'wasm');
  await fs.writeFile(path.join(root, 'private.txt'), 'private');
  const server = await serveFrontend(frontend, 12345);
  t.after(() => { server.closeAllConnections(); server.close(); });
  const base = `http://127.0.0.1:${server.address().port}`;
  const wasm = await fetch(`${base}/test.wasm`);
  assert.equal(wasm.headers.get('content-type'), 'application/wasm');
  assert.match(wasm.headers.get('content-security-policy'), /ws:\/\/127.0.0.1:12345/);
  assert.equal(await wasm.text(), 'wasm');
  assert.equal(await (await fetch(base, { method: 'HEAD' })).text(), '');
  assert.equal((await fetch(base, { method: 'POST' })).status, 405);
  assert.equal((await fetch(`${base}/missing`)).status, 404);
  assert.equal((await fetch(`${base}/%ZZ`)).status, 400);
  const rawStatus = (requestPath, host) => new Promise((resolve, reject) => {
    const req = http.get({ hostname: '127.0.0.1', port: server.address().port, path: requestPath, headers: { Host: host } }, res => {
      res.resume(); resolve(res.statusCode);
    });
    req.on('error', reject);
  });
  assert.equal(await rawStatus('/..%2fprivate.txt', `127.0.0.1:${server.address().port}`), 403);
  assert.equal(await rawStatus('/', 'example.com'), 403);
});

test('地形ファイル不足はサーバー起動前に検出する', async () => {
  await assert.rejects(validateTerrain(__dirname));
});

test('実サーバーは自動ポートで並行起動し、HTTP Range・WS接続・終了を行える', async t => {
  const serverExe = process.env.SIM3DVIEW_SERVER_EXE || developmentServer();
  // 通信検証用の最小データを一時生成し、実地形の有無に依存させない。
  const terrainDir = await fs.mkdtemp(path.join(os.tmpdir(), 'sim3dview-terrain-'));
  t.after(() => fs.rm(terrainDir, { recursive: true, force: true }));
  await fs.mkdir(path.join(terrainDir, 'tiles'));
  await fs.writeFile(path.join(terrainDir, 'metadata.json'), JSON.stringify({
    geodetic_bounds: { min_lat: 20, max_lat: 50, min_lon: 120, max_lon: 150 },
  }));
  await fs.writeFile(path.join(terrainDir, 'tile_index.json'), '[]');
  await fs.writeFile(path.join(terrainDir, 'base.bin'), Buffer.alloc(32));
  const frontendDir = await fs.mkdtemp(path.join(os.tmpdir(), 'sim3dview-ui-'));
  t.after(() => fs.rm(frontendDir, { recursive: true, force: true }));
  await fs.writeFile(path.join(frontendDir, 'index.html'), '<title>test</title>');
  const instances = [0, 1].map(() => new DesktopRuntime({ serverExe, terrainDir, frontendDir }));
  t.after(async () => { await Promise.all(instances.map(instance => instance.stop())); });
  await Promise.all(instances.map(instance => instance.start()));
  assert.notEqual(instances[0].backendPort, instances[1].backendPort);
  for (const instance of instances) {
    const base = `http://127.0.0.1:${instance.backendPort}`;
    const response = await fetch(`${base}/terrain/base.bin`, { headers: { Range: 'bytes=0-15' } });
    assert.equal(response.status, 206);
    assert.equal((await response.arrayBuffer()).byteLength, 16);
    await new Promise((resolve, reject) => {
      const ws = new WebSocket(`ws://127.0.0.1:${instance.backendPort}/sim`);
      const timer = setTimeout(() => { ws.close(); reject(new Error('WSタイムアウト')); }, 3000);
      ws.onmessage = () => { clearTimeout(timer); ws.close(); resolve(); };
      ws.onerror = error => { clearTimeout(timer); reject(error); };
    });
    await instance.stop();
    await assert.rejects(fetch(`${base}/terrain/metadata.json`));
  }
});
