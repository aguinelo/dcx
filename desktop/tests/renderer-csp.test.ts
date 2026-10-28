import assert from 'node:assert/strict';
import { readFile, readdir } from 'node:fs/promises';
import path from 'node:path';
import test from 'node:test';
import { fileURLToPath } from 'node:url';

const rendererDir = path.join(path.dirname(fileURLToPath(import.meta.url)), '..', 'src', 'desktop', 'renderer');

test('the renderer CSP forbids inline style attributes', async () => {
  const html = await readFile(path.join(rendererDir, 'index.html'), 'utf8');
  const styleSrc = /style-src ([^;"]*)/.exec(html)?.[1] ?? '';
  assert.ok(!styleSrc.includes("'unsafe-inline'"), 'this test assumes inline styles are blocked');
});

test('renderer markup never relies on inline style attributes, which the CSP drops', async () => {
  const offenders: string[] = [];
  for (const name of await readdir(rendererDir)) {
    if (!name.endsWith('.ts') && !name.endsWith('.html')) continue;
    const lines = (await readFile(path.join(rendererDir, name), 'utf8')).split('\n');
    lines.forEach((line, index) => {
      if (/\sstyle=["'$]/.test(line)) offenders.push(`${name}:${index + 1}: ${line.trim()}`);
    });
  }
  assert.deepEqual(offenders, [], 'use element.style or a class instead');
});
