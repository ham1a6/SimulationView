'use strict';

const path = require('node:path');

function platformLayout(platform = process.platform) {
  if (platform === 'win32') return {
    server: 'sim_server.exe', electron: 'electron.exe', app: 'Sim3dView.exe',
  };
  if (platform === 'linux') return {
    server: 'sim_server', electron: 'electron', app: 'Sim3dView',
  };
  throw new Error(`未対応のOSです: ${platform}`);
}

function developmentServer(configuration = 'Debug', platform = process.platform) {
  const layout = platformLayout(platform);
  return path.resolve(__dirname, '../sim_server/build',
    ...(platform === 'win32' ? [configuration] : []), layout.server);
}

module.exports = { platformLayout, developmentServer };
