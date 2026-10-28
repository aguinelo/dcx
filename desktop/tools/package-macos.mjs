import { spawnSync } from 'node:child_process';
import { access } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';

const desktopRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), '..');
const repositoryRoot = path.resolve(desktopRoot, '..');
const binaries = ['dcx', 'dcxd'].map((name) => path.join(repositoryRoot, 'target', 'release', name));

function run(command, args, cwd) {
  const result = spawnSync(command, args, { cwd, env: process.env, stdio: 'inherit' });
  if (result.error) throw result.error;
  if (result.status !== 0) process.exit(result.status ?? 1);
}

if (process.platform !== 'darwin') {
  throw new Error('A criação do app macOS precisa rodar no macOS.');
}

run('cargo', ['build', '--release', '--manifest-path', path.join(repositoryRoot, 'Cargo.toml'), '-p', 'dcx-cli', '--bins'], repositoryRoot);
for (const binary of binaries) await access(binary);
run('pnpm', ['run', 'build'], desktopRoot);
run('pnpm', ['exec', 'electron-builder', '--mac', 'dmg'], desktopRoot);

console.log(`\nDMG criado em ${path.join(desktopRoot, 'release')}`);
