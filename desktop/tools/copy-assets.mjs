/** Copy non-TypeScript renderer assets into dist after tsc. */
import { cpSync, mkdirSync } from 'node:fs';
import path from 'node:path';

const from = path.resolve('src/desktop');
const to = path.resolve('dist/desktop');
mkdirSync(path.join(to, 'renderer'), { recursive: true });

for (const file of ['index.html', 'app.css', 'fonts.css']) {
  cpSync(path.join(from, 'renderer', file), path.join(to, 'renderer', file));
}
// Web fonts are binary, so they ride along as a directory rather than a
// file list. tsc only emits .js/.d.ts/.map, so there is nothing to clash with.
cpSync(path.join(from, 'renderer', 'fonts'), path.join(to, 'renderer', 'fonts'), {
  recursive: true,
});
for (const file of ['preload.cjs']) {
  cpSync(path.join(from, file), path.join(to, file));
}

console.log('assets do renderer copiados');
