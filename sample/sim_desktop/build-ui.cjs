'use strict';

const { spawnSync } = require('node:child_process');
const { existsSync } = require('node:fs');
const os = require('node:os');
const path = require('node:path');

const env = { ...process.env, NO_COLOR: 'true' };
// Windowsのrustupリンクが利用できない環境でも既存の回避策を維持する。
if (process.platform === 'win32') {
  const bin = path.join(os.homedir(), '.rustup/toolchains/stable-x86_64-pc-windows-msvc/bin');
  const pathKey = Object.keys(env).find(key => key.toLowerCase() === 'path') || 'PATH';
  if (existsSync(path.join(bin, 'cargo.exe'))) env[pathKey] = `${bin}${path.delimiter}${env[pathKey] || ''}`;
}
const result = spawnSync('trunk', ['build', '--release', '--dist', path.join(__dirname, 'out/frontend')], {
  cwd: path.resolve(__dirname, '../sim_frontend'), env, stdio: 'inherit',
});
if (result.error) console.error(`UIのビルドを開始できません: ${result.error.message}`);
process.exitCode = result.status ?? 1;
