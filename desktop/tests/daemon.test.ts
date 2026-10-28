import assert from 'node:assert/strict';
import path from 'node:path';
import test from 'node:test';
import { resolveDcxCommand, shouldManageDcx } from '../src/desktop/daemon.ts';

test('packaged desktop resolves the bundled dcxd daemon', () => {
  const result = resolveDcxCommand({
    packaged: true,
    resourcesPath: '/Applications/dcode.app/Contents/Resources',
    desktopRoot: '/repo/desktop',
    platform: 'darwin',
    env: {},
  });

  assert.equal(result.command, '/Applications/dcode.app/Contents/Resources/bin/dcxd');
  assert.deepEqual(result.args, ['--bind', '127.0.0.1:8787']);
});

test('development desktop resolves the workspace debug daemon', () => {
  const result = resolveDcxCommand({
    packaged: false,
    resourcesPath: '/unused',
    desktopRoot: '/repo/desktop',
    platform: 'darwin',
    env: {},
  });

  assert.equal(result.command, path.join('/repo', 'target', 'debug', 'dcxd'));
});

test('DCX_DAEMON_BIN overrides the packaged or development daemon', () => {
  const result = resolveDcxCommand({
    packaged: true,
    resourcesPath: '/unused',
    desktopRoot: '/unused',
    env: { DCX_DAEMON_BIN: '/opt/dcx/bin/dcxd' },
  });

  assert.equal(result.command, '/opt/dcx/bin/dcxd');
});

test('custom endpoints and explicit opt-out are not auto-managed', () => {
  assert.equal(shouldManageDcx({ DCX_URL: 'http://127.0.0.1:9999' }), false);
  assert.equal(shouldManageDcx({ DCX_MANAGE_DAEMON: '0' }), false);
  assert.equal(shouldManageDcx({}), true);
});
