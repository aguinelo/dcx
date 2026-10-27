import assert from 'node:assert/strict';
import path from 'node:path';
import test from 'node:test';
import { resolveHarnessCommand, shouldManageHarness } from '../src/desktop/daemon.ts';

test('packaged desktop resolves the bundled Harness CLI', () => {
  const result = resolveHarnessCommand({
    packaged: true,
    resourcesPath: '/Applications/Harness.app/Contents/Resources',
    desktopRoot: '/repo/desktop',
    platform: 'darwin',
    env: {},
  });

  assert.equal(result.command, '/Applications/Harness.app/Contents/Resources/bin/harness');
  assert.deepEqual(result.args, ['serve', '--bind', '127.0.0.1:8787']);
});

test('development desktop resolves the workspace debug binary', () => {
  const result = resolveHarnessCommand({
    packaged: false,
    resourcesPath: '/unused',
    desktopRoot: '/repo/desktop',
    platform: 'darwin',
    env: {},
  });

  assert.equal(result.command, path.join('/repo', 'target', 'debug', 'harness'));
});

test('HARNESS_BIN overrides the packaged or development binary', () => {
  const result = resolveHarnessCommand({
    packaged: true,
    resourcesPath: '/unused',
    desktopRoot: '/unused',
    env: { HARNESS_BIN: '/opt/harness/bin/harness' },
  });

  assert.equal(result.command, '/opt/harness/bin/harness');
});

test('custom endpoints and explicit opt-out are not auto-managed', () => {
  assert.equal(shouldManageHarness({ HARNESS_URL: 'http://127.0.0.1:9999' }), false);
  assert.equal(shouldManageHarness({ HARNESS_MANAGE_DAEMON: '0' }), false);
  assert.equal(shouldManageHarness({}), true);
});
