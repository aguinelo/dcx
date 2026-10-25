import assert from 'node:assert/strict';
import { mkdirSync, mkdtempSync, symlinkSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { after, describe, it } from 'node:test';
import {
  SandboxError,
  assertNoDotDot,
  assertNotAbsolute,
  assertPatternLegal,
  isInside,
  permittedRoots,
  resolveUnderRoot,
} from '../src/fs/sandbox.ts';

const scratch = mkdtempSync(path.join(tmpdir(), 'repl-sandbox-'));
after(() => scratch);

function makeWorkdir(name: string): string {
  const dir = path.join(scratch, name);
  mkdirSync(dir, { recursive: true });
  return dir;
}

describe('isInside', () => {
  it('accepts the root itself', () => {
    assert.equal(isInside('/tmp/root', '/tmp/root'), true);
  });

  it('accepts descendants', () => {
    assert.equal(isInside('/tmp/root', '/tmp/root/a/b'), true);
  });

  it('rejects the parent', () => {
    assert.equal(isInside('/tmp/root', '/tmp'), false);
  });

  it('rejects a sibling sharing a name prefix', () => {
    // Guards the classic `startsWith` bug.
    assert.equal(isInside('/tmp/root', '/tmp/rootevil'), false);
  });

  it('rejects a traversal back out', () => {
    assert.equal(isInside('/tmp/root', '/tmp/root/a/../..'), false);
  });
});

describe('assertNoDotDot', () => {
  it('rejects a bare parent segment', () => {
    assert.throws(
      () => assertNoDotDot('../etc/passwd'),
      (e: unknown) => {
        assert.ok(e instanceof SandboxError);
        assert.equal(e.code, 'DOTDOT_NOT_PERMITTED');
        return true;
      },
    );
  });

  it('rejects an embedded parent segment', () => {
    assert.throws(() => assertNoDotDot('a/../../b'), SandboxError);
  });

  it('rejects a backslash parent segment', () => {
    assert.throws(() => assertNoDotDot('a\\..\\b'), SandboxError);
  });

  it('accepts a name that merely starts with dots', () => {
    assert.doesNotThrow(() => assertNoDotDot('..foo/.bar/baz'));
  });
});

describe('assertNotAbsolute', () => {
  it('rejects posix absolute', () => {
    assert.throws(() => assertNotAbsolute('/etc/passwd'), SandboxError);
  });

  it('rejects windows drive absolute', () => {
    assert.throws(() => assertNotAbsolute('C:\\Windows'), SandboxError);
  });
});

describe('resolveUnderRoot', () => {
  const workdir = makeWorkdir('workdir');
  writeFileSync(path.join(workdir, 'inside.txt'), 'hello');

  it('resolves a plain relative path', () => {
    const resolved = resolveUnderRoot('inside.txt', { workdir });
    assert.equal(resolved, path.join(workdir, 'inside.txt'));
  });

  it('resolves a nested relative path', () => {
    const resolved = resolveUnderRoot('a/b/c.txt', { workdir });
    assert.equal(isInside(workdir, resolved), true);
  });

  it('refuses an absolute path', () => {
    assert.throws(
      () => resolveUnderRoot('/etc/passwd', { workdir }),
      (e: unknown) => {
        assert.ok(e instanceof SandboxError);
        assert.equal(e.code, 'ABSOLUTE_NOT_PERMITTED');
        return true;
      },
    );
  });

  it('refuses a traversal escape', () => {
    assert.throws(
      () => resolveUnderRoot('../../etc/passwd', { workdir }),
      (e: unknown) => {
        assert.ok(e instanceof SandboxError);
        assert.equal(e.code, 'DOTDOT_NOT_PERMITTED');
        return true;
      },
    );
  });

  it('accepts an absolute path inside an explicitly allowed root', () => {
    // This is what makes --add-dir reachable without ever permitting `..`.
    const extra = makeWorkdir('extra-root');
    writeFileSync(path.join(extra, 'ok.txt'), 'ok');
    const resolved = resolveUnderRoot(path.join(extra, 'ok.txt'), {
      workdir,
      allowedRoots: [extra],
    });
    assert.equal(resolved, path.join(extra, 'ok.txt'));
  });

  it('resolves a relative path against the workdir even when roots are allowed', () => {
    const extra = makeWorkdir('extra-root-rel');
    writeFileSync(path.join(extra, 'ok.txt'), 'ok');
    const resolved = resolveUnderRoot('ok.txt', { workdir, allowedRoots: [extra] });
    assert.equal(resolved, path.join(workdir, 'ok.txt'));
  });

  it('accepts an absolute path inside the workdir itself', () => {
    const resolved = resolveUnderRoot(path.join(workdir, 'inside.txt'), { workdir });
    assert.equal(resolved, path.join(workdir, 'inside.txt'));
  });

  it('refuses an absolute path outside every permitted root', () => {
    const extra = makeWorkdir('extra-root-3');
    assert.throws(
      () =>
        resolveUnderRoot(path.join(scratch, 'elsewhere.txt'), { workdir, allowedRoots: [extra] }),
      (e: unknown) => {
        assert.ok(e instanceof SandboxError);
        assert.equal(e.code, 'ABSOLUTE_NOT_PERMITTED');
        return true;
      },
    );
  });

  it('still refuses escapes even when extra roots are allowed', () => {
    const extra = makeWorkdir('extra-root-2');
    assert.throws(() =>
      resolveUnderRoot('../../../etc/passwd', { workdir, allowedRoots: [extra] }),
    );
  });

  it('detects a symlinked parent that escapes the root', () => {
    // The attack: a symlink inside the workdir pointing outside it. No `..` is
    // used, so only the real-path check can catch this.
    const outside = makeWorkdir('outside');
    writeFileSync(path.join(outside, 'secret.txt'), 'secret');

    const linkDir = path.join(workdir, 'link');
    symlinkSync(outside, linkDir, 'dir');

    assert.throws(
      () => resolveUnderRoot('link/secret.txt', { workdir }),
      (e: unknown) => {
        assert.ok(e instanceof SandboxError);
        assert.equal(e.code, 'IS_A_SYMLINK');
        return true;
      },
    );
  });

  it('detects a symlink to an allowed root as legal', () => {
    const outside = makeWorkdir('outside-allowed');
    writeFileSync(path.join(outside, 'fine.txt'), 'fine');
    const linkDir = path.join(workdir, 'link-ok');
    symlinkSync(outside, linkDir, 'dir');

    const resolved = resolveUnderRoot('link-ok/fine.txt', {
      workdir,
      allowedRoots: [outside],
    });
    assert.equal(resolved, path.join(linkDir, 'fine.txt'));
  });

  it('allows a not-yet-existing path under a valid existing parent', () => {
    const resolved = resolveUnderRoot('new-dir/new.txt', { workdir });
    assert.equal(isInside(workdir, resolved), true);
  });
});

describe('assertPatternLegal', () => {
  it('accepts a normal glob', () => {
    assert.doesNotThrow(() => assertPatternLegal('src/**/*.ts'));
  });

  it('rejects an absolute glob', () => {
    assert.throws(() => assertPatternLegal('/tmp/**'), SandboxError);
  });

  it('rejects a glob containing a parent segment', () => {
    assert.throws(() => assertPatternLegal('../**'), SandboxError);
  });

  it('rejects an empty pattern', () => {
    assert.throws(() => assertPatternLegal('   '), SandboxError);
  });
});

describe('permittedRoots', () => {
  it('always includes the workdir first', () => {
    const roots = permittedRoots({ workdir: '/tmp/w', allowedRoots: ['/tmp/x'] });
    assert.deepEqual(roots, [path.resolve('/tmp/w'), path.resolve('/tmp/x')]);
  });
});
