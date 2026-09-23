'use strict';

const { test } = require('node:test');
const assert = require('node:assert/strict');
const path = require('node:path');
const { platformLayout, developmentServer } = require('../platform.cjs');

test('Linuxは単一構成、Windowsは構成別のサーバーを起動する', () => {
  const build = path.resolve(__dirname, '../../sim_server/build');
  assert.equal(developmentServer('Debug', 'linux'), path.join(build, 'sim_server'));
  assert.equal(developmentServer('Release', 'linux'), path.join(build, 'sim_server'));
  assert.equal(developmentServer('Debug', 'win32'), path.join(build, 'Debug/sim_server.exe'));
  assert.equal(developmentServer('Release', 'win32'), path.join(build, 'Release/sim_server.exe'));
  assert.equal(platformLayout('linux').app, 'Sim3dView');
  assert.equal(platformLayout('win32').app, 'Sim3dView.exe');
  assert.throws(() => platformLayout('darwin'), /未対応/);
});
