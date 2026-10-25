import { type Dirent, readFileSync, readdirSync, statSync } from 'node:fs';
import path from 'node:path';
import { z } from 'zod';
import { SandboxError, assertPatternLegal, isInside, resolveUnderRoot } from '../fs/sandbox.ts';
import { type Tool, type ToolContext, ToolError } from './types.ts';

/** Directories that are never worth walking in a source tree. */
const DEFAULT_IGNORED_DIRS = new Set([
  'node_modules',
  '.git',
  'dist',
  'build',
  '.next',
  '.turbo',
  '.cache',
  'coverage',
  '.venv',
]);

const MAX_FILES = 20_000;
const MAX_MATCHES = 500;

interface WalkEntry {
  absolute: string;
  relative: string;
  mtimeMs: number;
  size: number;
}

function safeReaddir(dir: string): Dirent[] {
  try {
    return readdirSync(dir, { withFileTypes: true });
  } catch {
    return [];
  }
}

function toWalkEntry(root: string, absolute: string): WalkEntry | null {
  try {
    const stat = statSync(absolute);
    return {
      absolute,
      relative: path.relative(root, absolute),
      mtimeMs: stat.mtimeMs,
      size: stat.size,
    };
  } catch {
    return null;
  }
}

/** Expand one directory into the queue (subdirs) and the output (files). */
function collectFromDir(root: string, dir: string, queue: string[], out: WalkEntry[]): void {
  for (const entry of safeReaddir(dir)) {
    const absolute = path.join(dir, entry.name);

    if (entry.isDirectory()) {
      if (!DEFAULT_IGNORED_DIRS.has(entry.name)) queue.push(absolute);
      continue;
    }
    if (!entry.isFile()) continue;

    const walked = toWalkEntry(root, absolute);
    if (walked) out.push(walked);
  }
}

/**
 * Breadth-first walk that skips heavy, well-known build directories.
 *
 * A global file cap keeps a runaway tree from stalling the agent.
 */
function walk(root: string): WalkEntry[] {
  const out: WalkEntry[] = [];
  const queue: string[] = [root];

  while (queue.length > 0 && out.length < MAX_FILES) {
    collectFromDir(root, queue.shift() as string, queue, out);
  }

  return out;
}

/** Translate a glob into an anchored regex. Only the subset the tool documents. */
export function globToRegExp(glob: string): RegExp {
  let source = '';

  for (let i = 0; i < glob.length; i += 1) {
    const char = glob[i] as string;
    const next = glob[i + 1];

    if (char === '*' && next === '*') {
      if (glob[i + 2] === '/') {
        source += '(?:.*/)?';
        i += 2;
      } else {
        source += '.*';
        i += 1;
      }
      continue;
    }
    if (char === '*') {
      source += '[^/]*';
      continue;
    }
    if (char === '?') {
      source += '[^/]';
      continue;
    }
    if ('.+^${}()|[]\\'.includes(char)) {
      source += `\\${char}`;
      continue;
    }
    source += char;
  }

  return new RegExp(`^${source}$`);
}

function searchRoot(ctx: ToolContext, target: string | undefined): string {
  if (!target) return path.resolve(ctx.sandbox.workdir);
  try {
    return resolveUnderRoot(target, ctx.sandbox);
  } catch (error) {
    if (error instanceof SandboxError) throw new ToolError(`glob/grep: ${error.message}`);
    throw error;
  }
}

const GlobInput = z.object({
  pattern: z.string().min(1),
  path: z.string().optional(),
});

export const globTool: Tool<typeof GlobInput> = {
  name: 'glob',
  description:
    'Match files under a directory against a glob pattern. Results are mtime-sorted, newest first.',
  effect: 'read',
  schema: GlobInput,

  async run(input, ctx) {
    try {
      assertPatternLegal(input.pattern);
    } catch (error) {
      throw new ToolError(`glob: ${error instanceof SandboxError ? error.message : String(error)}`);
    }

    const root = searchRoot(ctx, input.path);
    const matcher = globToRegExp(input.pattern);

    const matches = walk(root)
      .filter((entry) => {
        const normalized = entry.relative.split(path.sep).join('/');
        return matcher.test(normalized);
      })
      .sort((a, b) => b.mtimeMs - a.mtimeMs)
      .slice(0, MAX_MATCHES);

    if (matches.length === 0) return `no files matched ${input.pattern}`;
    return matches.map((entry) => entry.relative).join('\n');
  },
};

const GrepInput = z.object({
  pattern: z.string().min(1),
  path: z.string().optional(),
  glob: z.string().optional(),
  ignore_case: z.boolean().optional(),
  max_per_file: z.number().int().positive().optional(),
});

/** Read a file as text, or return null for anything unreadable or binary. */
function readTextOrNull(absolute: string): string | null {
  try {
    const buffer = readFileSync(absolute);
    if (buffer.subarray(0, 8000).includes(0)) return null;
    return buffer.toString('utf8');
  } catch {
    return null;
  }
}

/** Matching lines in one file, capped so a huge file cannot flood the context. */
function scanFile(entry: WalkEntry, matcher: RegExp, maxPerFile: number, budget: number): string[] {
  const content = readTextOrNull(entry.absolute);
  if (content === null) return [];

  const found: string[] = [];
  const lines = content.split('\n');

  for (let i = 0; i < lines.length; i += 1) {
    const line = lines[i] as string;
    if (!matcher.test(line)) continue;
    found.push(`${entry.relative}:${i + 1}:${line.trim().slice(0, 400)}`);
    if (found.length >= maxPerFile || found.length >= budget) break;
  }

  return found;
}

function compileMatcher(pattern: string, ignoreCase: boolean | undefined): RegExp {
  try {
    return new RegExp(pattern, ignoreCase ? 'i' : '');
  } catch (error) {
    throw new ToolError(
      `grep: invalid regex: ${error instanceof Error ? error.message : String(error)}`,
    );
  }
}

/** Walk the tree and collect matches until the global budget runs out. */
function collectHits(
  root: string,
  matcher: RegExp,
  fileMatcher: RegExp | undefined,
  maxPerFile: number,
): string[] {
  const hits: string[] = [];

  for (const entry of walk(root)) {
    const budget = MAX_MATCHES - hits.length;
    if (budget <= 0) break;

    if (fileMatcher) {
      const normalized = entry.relative.split(path.sep).join('/');
      if (!fileMatcher.test(normalized)) continue;
    }

    hits.push(...scanFile(entry, matcher, maxPerFile, budget));
  }

  return hits;
}

export const grepTool: Tool<typeof GrepInput> = {
  name: 'grep',
  description: 'Search file contents for a regex.',
  effect: 'read',
  schema: GrepInput,

  async run(input, ctx) {
    const matcher = compileMatcher(input.pattern, input.ignore_case);
    const root = searchRoot(ctx, input.path);
    const fileMatcher = input.glob ? globToRegExp(input.glob) : undefined;

    const hits = collectHits(root, matcher, fileMatcher, input.max_per_file ?? 10);

    if (hits.length === 0) return `no matches for ${input.pattern}`;
    const truncated = hits.length >= MAX_MATCHES ? '\n[output truncated]' : '';
    return hits.join('\n') + truncated;
  },
};

export { isInside };
