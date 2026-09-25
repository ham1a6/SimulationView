'use strict';

const { spawn } = require('node:child_process');
const { createServer } = require('node:http');
const { createReadStream } = require('node:fs');
const { realpath, stat } = require('node:fs/promises');
const path = require('node:path');

const mime = {
  '.html': 'text/html; charset=utf-8', '.js': 'text/javascript; charset=utf-8',
  '.wasm': 'application/wasm', '.css': 'text/css; charset=utf-8',
  '.json': 'application/json', '.glb': 'model/gltf-binary',
  '.png': 'image/png', '.svg': 'image/svg+xml', '.ico': 'image/x-icon',
};

async function validateTerrain(directory) {
  for (const name of ['metadata.json', 'tile_index.json', 'base.bin', 'tiles']) {
    const entry = await stat(path.join(directory, name));
    if (name === 'tiles' ? !entry.isDirectory() : !entry.isFile()) {
      throw new Error(`地形データが不正です: ${name}`);
    }
  }
}

async function serveFrontend(directory, backendPort) {
  const root = await realpath(directory);
  if (!(await stat(path.join(root, 'index.html'))).isFile()) {
    throw new Error('UIがありません。npm run build:ui を実行してください。');
  }
  const server = createServer(async (req, res) => {
    try {
      // DNS rebindingと、公開フォルダー外へのパストラバーサルを拒否する。
      if (req.headers.host !== `127.0.0.1:${server.address().port}`) {
        res.writeHead(403).end(); return;
      }
      if (!['GET', 'HEAD'].includes(req.method)) {
        res.writeHead(405, { Allow: 'GET, HEAD' }).end(); return;
      }
      const pathname = decodeURIComponent(new URL(req.url, 'http://localhost').pathname);
      const requested = pathname === '/' ? 'index.html' : pathname.slice(1);
      const candidate = path.resolve(root, requested);
      const candidateRelative = path.relative(root, candidate);
      if (candidateRelative.startsWith('..') || path.isAbsolute(candidateRelative)) {
        res.writeHead(403).end(); return;
      }
      const file = await realpath(candidate);
      const relative = path.relative(root, file);
      if (relative.startsWith('..') || path.isAbsolute(relative)) {
        res.writeHead(403).end(); return;
      }
      const info = await stat(file);
      if (!info.isFile()) { res.writeHead(404).end(); return; }
      res.writeHead(200, {
        'Content-Type': mime[path.extname(file)] || 'application/octet-stream',
        'Content-Length': info.size,
        'Cache-Control': 'no-store',
        'X-Content-Type-Options': 'nosniff',
        // TrunkのインラインmoduleとLeptosの動的styleを許可する。
        'Content-Security-Policy': `default-src 'self'; script-src 'self' 'unsafe-inline' 'wasm-unsafe-eval'; style-src 'self' 'unsafe-inline'; img-src 'self' data: blob:; connect-src 'self' http://127.0.0.1:${backendPort} ws://127.0.0.1:${backendPort}; object-src 'none'; base-uri 'none'; frame-ancestors 'none'`,
      });
      if (req.method === 'HEAD') { res.end(); return; }
      const stream = createReadStream(file);
      stream.on('error', () => res.destroy());
      res.on('close', () => stream.destroy());
      stream.pipe(res);
    } catch (error) {
      res.writeHead(error instanceof URIError ? 400 : 404).end();
    }
  });
  await new Promise((resolve, reject) => {
    server.once('error', reject);
    server.listen(0, '127.0.0.1', resolve);
  });
  return server;
}

class DesktopRuntime {
  constructor({ serverExe, terrainDir, frontendDir, onFailure = () => {} }) {
    Object.assign(this, { serverExe, terrainDir, frontendDir, onFailure });
    this.stopping = false;
    this.log = '';
  }

  async start() {
    await validateTerrain(this.terrainDir);
    if (this.stopping) throw new Error('起動を中止しました。');
    this.child = spawn(this.serverExe, ['0', '--host', '127.0.0.1', '--terrain-dir', this.terrainDir], {
      cwd: path.dirname(this.serverExe), windowsHide: true, stdio: ['ignore', 'pipe', 'pipe'],
    });
    this.child.stderr.on('data', chunk => { this.log = (this.log + chunk).slice(-8192); });
    const port = await new Promise((resolve, reject) => {
      const timer = setTimeout(() => reject(new Error(`サーバー起動がタイムアウトしました。\n${this.log}`)), 15000);
      let pending = '';
      let ready = false;
      this.child.once('error', error => { clearTimeout(timer); reject(error); });
      this.child.once('exit', (code, signal) => {
        clearTimeout(timer);
        const error = new Error(`シミュレーションサーバーが終了しました (${code ?? signal})。\n${this.log}`);
        if (!ready) reject(error);
        else if (!this.stopping) this.onFailure(error);
      });
      this.child.stdout.on('data', chunk => {
        this.log = (this.log + chunk).slice(-8192);
        pending += chunk;
        const lines = pending.split(/\r?\n/);
        pending = lines.pop().slice(-8192);
        for (const line of lines) {
          const match = /^SIM3DVIEW_READY ([0-9]+)(?: [A-Za-z0-9_-]+)?$/.exec(line);
          if (!ready && match && Number(match[1]) > 0 && Number(match[1]) <= 65535) {
            ready = true;
            clearTimeout(timer);
            resolve(Number(match[1]));
          }
        }
      });
    });
    if (this.stopping) throw new Error('起動を中止しました。');
    this.backendPort = port;
    this.http = await serveFrontend(this.frontendDir, port);
    if (this.stopping) {
      this.http.closeAllConnections(); this.http.close();
      throw new Error('起動を中止しました。');
    }
    this.url = `http://127.0.0.1:${this.http.address().port}/?sim_port=${port}`;
    return this.url;
  }

  async stop() {
    this.stopping = true;
    if (this.http) { this.http.closeAllConnections(); this.http.close(); }
    const child = this.child;
    if (!child || child.exitCode !== null || child.signalCode !== null || !child.pid) return;
    await new Promise(resolve => {
      const timer = setTimeout(() => child.kill('SIGKILL'), 2000);
      child.once('exit', () => { clearTimeout(timer); resolve(); });
      child.kill();
    });
  }
}

module.exports = { DesktopRuntime, serveFrontend, validateTerrain };
