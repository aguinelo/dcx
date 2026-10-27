import assert from 'node:assert/strict';
import { mkdtempSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { describe, it } from 'node:test';
import { PersistentShell } from '../src/tools/shell.ts';

const workdir = mkdtempSync(path.join(tmpdir(), 'repl-shell-'));
writeFileSync(path.join(workdir, 'marker.txt'), 'here');

function makeShell() {
  const shell = new PersistentShell(workdir);
  return shell;
}

describe('PersistentShell', () => {
  it('runs a command and returns stdout with exit code 0', async () => {
    const shell = makeShell();
    try {
      const result = await shell.run('echo hello');
      assert.equal(result.exitCode, 0);
      assert.equal(result.stdout.trim(), 'hello');
    } finally {
      shell.close();
    }
  });

  it('reports a non-zero exit code', async () => {
    const shell = makeShell();
    try {
      // Subshell, so the failure is reported without ending the session.
      const result = await shell.run('(exit 3)');
      assert.equal(result.exitCode, 3);
    } finally {
      shell.close();
    }
  });

  it('treats a bare exit as ending the session, then recovers on the next call', async () => {
    const shell = makeShell();
    try {
      const result = await shell.run('exit 3');
      assert.notEqual(result.exitCode, 0);
      // The next call respawns, so a caller that exits the shell loses state
      // but not the ability to continue.
      const next = await shell.run('echo alive');
      assert.equal(next.stdout.trim(), 'alive');
    } finally {
      shell.close();
    }
  });

  it('keeps cwd across calls', async () => {
    const shell = makeShell();
    try {
      await shell.run('cd /tmp');
      const result = await shell.run('pwd');
      assert.notEqual(result.stdout.trim(), workdir);
    } finally {
      shell.close();
    }
  });

  it('keeps exported environment variables across calls', async () => {
    const shell = makeShell();
    try {
      await shell.run('export REPL_TEST_VAR=kept');
      const result = await shell.run('echo "$REPL_TEST_VAR"');
      assert.equal(result.stdout.trim(), 'kept');
    } finally {
      shell.close();
    }
  });

  it('resolves files in the workdir on the first call', async () => {
    const shell = makeShell();
    try {
      const result = await shell.run('ls marker.txt');
      assert.equal(result.exitCode, 0);
      assert.match(result.stdout, /marker\.txt/);
    } finally {
      shell.close();
    }
  });

  it('serializes concurrent calls without interleaving output', async () => {
    const shell = makeShell();
    try {
      const results = await Promise.all([
        shell.run('echo one'),
        shell.run('echo two'),
        shell.run('echo three'),
      ]);
      const outputs = results.map((r) => r.stdout.trim());
      assert.deepEqual(outputs.sort(), ['one', 'three', 'two']);
      for (const result of results) assert.equal(result.exitCode, 0);
    } finally {
      shell.close();
    }
  });

  it('keeps output of a compound command together', async () => {
    const shell = makeShell();
    try {
      const result = await shell.run('echo a && echo b');
      assert.equal(result.exitCode, 0);
      assert.deepEqual(result.stdout.trim().split('\n'), ['a', 'b']);
    } finally {
      shell.close();
    }
  });

  it('captures stderr alongside stdout', async () => {
    const shell = makeShell();
    try {
      const result = await shell.run('echo oops 1>&2');
      assert.match(result.stdout, /oops/);
    } finally {
      shell.close();
    }
  });

  it('times out a long running command', async () => {
    const shell = makeShell();
    try {
      const result = await shell.run('sleep 5', { timeoutMs: 150 });
      assert.equal(result.timedOut, true);
      const recovered = await shell.run('echo recovered');
      assert.equal(recovered.stdout.trim(), 'recovered');
    } finally {
      shell.close();
    }
  });

  it('caps captured command output', async () => {
    const shell = makeShell();
    try {
      const result = await shell.run("head -c 1200000 /dev/zero | tr '\\0' x");
      assert.match(result.stdout, /^\[saída truncada pelo limite de captura\]/);
      assert.ok(result.stdout.length <= 1_000_000 + 64);
    } finally {
      shell.close();
    }
  });

  it('still works after a restart', async () => {
    const shell = makeShell();
    try {
      await shell.run('export SHOULD_BE_GONE=1');
      shell.restart();
      const result = await shell.run('echo "[${SHOULD_BE_GONE:-}]"');
      assert.equal(result.stdout.trim(), '[]');
    } finally {
      shell.close();
    }
  });

  it('rejects work after close', async () => {
    const shell = makeShell();
    shell.close();
    await assert.rejects(shell.run('echo x'));
  });
});
