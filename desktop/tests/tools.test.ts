import assert from 'node:assert/strict';
import { mkdirSync, mkdtempSync, readFileSync, writeFileSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { describe, it } from 'node:test';
import { editTool, readTool, writeTool } from '../src/tools/fs.ts';
import { globToRegExp, globTool, grepTool } from '../src/tools/search.ts';
import { PersistentShell } from '../src/tools/shell.ts';
import type { ToolContext } from '../src/tools/types.ts';
import { ToolError } from '../src/tools/types.ts';

const root = mkdtempSync(path.join(tmpdir(), 'repl-tools-'));
const workdir = path.join(root, 'work');
const extra = path.join(root, 'extra');
mkdirSync(workdir, { recursive: true });
mkdirSync(extra, { recursive: true });

const shell = new PersistentShell(workdir);
const ctx: ToolContext = {
  sandbox: { workdir, allowedRoots: [extra] },
  shell,
};

function schemaOf(tool: { schema: { parse: (v: unknown) => unknown } }, raw: unknown) {
  return tool.schema.parse(raw) as never;
}

describe('readTool', () => {
  it('reads a file relative to the workdir', async () => {
    writeFileSync(path.join(workdir, 'a.txt'), 'line1\nline2\nline3\n');
    const out = await readTool.run(schemaOf(readTool, { file_path: 'a.txt' }), ctx);
    assert.equal(out, 'line1\nline2\nline3\n');
  });

  it('applies an inclusive 1-indexed view_range', async () => {
    writeFileSync(path.join(workdir, 'b.txt'), 'l1\nl2\nl3\nl4\nl5\n');
    const out = await readTool.run(
      schemaOf(readTool, { file_path: 'b.txt', view_range: [2, 4] }),
      ctx,
    );
    assert.equal(out, 'l2\nl3\nl4');
  });

  it('clamps a view_range past end of file', async () => {
    writeFileSync(path.join(workdir, 'c.txt'), 'only\n');
    const out = await readTool.run(
      schemaOf(readTool, { file_path: 'c.txt', view_range: [1, 99] }),
      ctx,
    );
    // The trailing newline is a real line break, so it comes back too.
    assert.equal(out, 'only\n');
  });

  it('refuses a missing file', async () => {
    await assert.rejects(
      readTool.run(schemaOf(readTool, { file_path: 'nope.txt' }), ctx),
      ToolError,
    );
  });

  it('refuses a directory', async () => {
    await assert.rejects(readTool.run(schemaOf(readTool, { file_path: '.' }), ctx), ToolError);
  });

  it('refuses a traversal escape', async () => {
    await assert.rejects(
      readTool.run(schemaOf(readTool, { file_path: '../extra/secret.txt' }), ctx),
      (error: unknown) => {
        assert.ok(error instanceof ToolError);
        assert.match(error.message, /escapes|"\.\."/);
        return true;
      },
    );
  });

  it('refuses an absolute path outside the roots', async () => {
    await assert.rejects(
      readTool.run(schemaOf(readTool, { file_path: '/etc/passwd' }), ctx),
      ToolError,
    );
  });

  it('allows an absolute path inside an allowed root', async () => {
    writeFileSync(path.join(extra, 'shared.txt'), 'shared content');
    const out = await readTool.run(
      schemaOf(readTool, { file_path: path.join(extra, 'shared.txt') }),
      ctx,
    );
    assert.equal(out, 'shared content');
  });

  it('rejects binary content', async () => {
    writeFileSync(path.join(workdir, 'bin.dat'), Buffer.from([0x00, 0x01, 0x02]));
    await assert.rejects(
      readTool.run(schemaOf(readTool, { file_path: 'bin.dat' }), ctx),
      ToolError,
    );
  });
});

describe('writeTool', () => {
  it('creates a file and its parent directories', async () => {
    await writeTool.run(
      schemaOf(writeTool, { file_path: 'nested/deep/new.txt', content: 'hi' }),
      ctx,
    );
    assert.equal(readFileSync(path.join(workdir, 'nested/deep/new.txt'), 'utf8'), 'hi');
  });

  it('overwrites an existing file', async () => {
    await writeTool.run(schemaOf(writeTool, { file_path: 'over.txt', content: 'v2' }), ctx);
    assert.equal(readFileSync(path.join(workdir, 'over.txt'), 'utf8'), 'v2');
  });

  it('refuses to write outside the sandbox', async () => {
    await assert.rejects(
      writeTool.run(schemaOf(writeTool, { file_path: '../evil.txt', content: 'x' }), ctx),
      ToolError,
    );
  });
});

describe('editTool', () => {
  it('replaces a unique occurrence', async () => {
    writeFileSync(path.join(workdir, 'e1.txt'), 'foo bar foo');
    const out = await editTool.run(
      schemaOf(editTool, { file_path: 'e1.txt', old_string: 'bar', new_string: 'baz' }),
      ctx,
    );
    assert.equal(readFileSync(path.join(workdir, 'e1.txt'), 'utf8'), 'foo baz foo');
    assert.match(out, /1 edited/);
  });

  it('refuses an ambiguous old_string', async () => {
    writeFileSync(path.join(workdir, 'e2.txt'), 'x x x');
    await assert.rejects(
      editTool.run(
        schemaOf(editTool, { file_path: 'e2.txt', old_string: 'x', new_string: 'y' }),
        ctx,
      ),
      (error: unknown) => {
        assert.ok(error instanceof ToolError);
        assert.match(error.message, /appears 3 times/);
        return true;
      },
    );
  });

  it('replaces every occurrence with replace_all', async () => {
    writeFileSync(path.join(workdir, 'e3.txt'), 'x x x');
    const out = await editTool.run(
      schemaOf(editTool, {
        file_path: 'e3.txt',
        old_string: 'x',
        new_string: 'y',
        replace_all: true,
      }),
      ctx,
    );
    assert.equal(readFileSync(path.join(workdir, 'e3.txt'), 'utf8'), 'y y y');
    assert.match(out, /3 edited/);
  });

  it('refuses an old_string that is absent', async () => {
    writeFileSync(path.join(workdir, 'e4.txt'), 'abc');
    await assert.rejects(
      editTool.run(
        schemaOf(editTool, { file_path: 'e4.txt', old_string: 'zzz', new_string: 'q' }),
        ctx,
      ),
      (error: unknown) => {
        assert.ok(error instanceof ToolError);
        assert.match(error.message, /old_string not found/);
        return true;
      },
    );
  });

  it('refuses to edit outside the sandbox', async () => {
    await assert.rejects(
      editTool.run(
        schemaOf(editTool, { file_path: '../evil.txt', old_string: 'a', new_string: 'b' }),
        ctx,
      ),
      ToolError,
    );
  });
});

describe('globToRegExp', () => {
  it('matches a single segment wildcard without crossing separators', () => {
    const re = globToRegExp('src/*.ts');
    assert.ok(re.test('src/a.ts'));
    assert.ok(!re.test('src/nested/a.ts'));
  });

  it('matches ** across segments', () => {
    const re = globToRegExp('src/**/*.ts');
    assert.ok(re.test('src/a.ts'));
    assert.ok(re.test('src/nested/deep/a.ts'));
  });

  it('anchors both ends', () => {
    const re = globToRegExp('a.ts');
    assert.ok(!re.test('xa.ts'));
    assert.ok(!re.test('a.tsx'));
  });
});

describe('globTool', () => {
  it('finds files by extension', async () => {
    mkdirSync(path.join(workdir, 'globset/sub'), { recursive: true });
    writeFileSync(path.join(workdir, 'globset/one.ts'), '');
    writeFileSync(path.join(workdir, 'globset/sub/two.ts'), '');
    writeFileSync(path.join(workdir, 'globset/three.md'), '');

    const out = await globTool.run(schemaOf(globTool, { pattern: 'globset/**/*.ts' }), ctx);
    assert.ok(out.includes('globset/one.ts'));
    assert.ok(out.includes('globset/sub/two.ts'));
    assert.ok(!out.includes('three.md'));
  });

  it('refuses an absolute pattern', async () => {
    await assert.rejects(globTool.run(schemaOf(globTool, { pattern: '/etc/**' }), ctx), ToolError);
  });

  it('refuses a parent segment in the pattern', async () => {
    await assert.rejects(globTool.run(schemaOf(globTool, { pattern: '../*' }), ctx), ToolError);
  });

  it('reports when nothing matches', async () => {
    const out = await globTool.run(schemaOf(globTool, { pattern: 'no-such-dir/*.rs' }), ctx);
    assert.match(out, /no files matched/);
  });
});

describe('grepTool', () => {
  it('finds matches with line numbers', async () => {
    writeFileSync(path.join(workdir, 'search1.ts'), 'const needle = 1;\nconst other = 2;\n');
    const out = await grepTool.run(schemaOf(grepTool, { pattern: 'needle' }), ctx);
    assert.match(out, /search1\.ts:1:const needle = 1/);
  });

  it('respects ignore_case', async () => {
    writeFileSync(path.join(workdir, 'search2.ts'), 'Needle\n');
    const insensitive = await grepTool.run(
      schemaOf(grepTool, { pattern: 'needle', ignore_case: true }),
      ctx,
    );
    assert.match(insensitive, /search2\.ts/);
  });

  it('filters by a glob', async () => {
    writeFileSync(path.join(workdir, 'search3.md'), 'needle\n');
    const out = await grepTool.run(schemaOf(grepTool, { pattern: 'needle', glob: '*.ts' }), ctx);
    assert.ok(!out.includes('search3.md'));
  });

  it('rejects an invalid regex', async () => {
    await assert.rejects(grepTool.run(schemaOf(grepTool, { pattern: '([' }), ctx), ToolError);
  });

  it('reports no matches', async () => {
    const out = await grepTool.run(schemaOf(grepTool, { pattern: 'zzzznotfoundzzzz' }), ctx);
    assert.match(out, /no matches/);
  });
});

shell.close();
