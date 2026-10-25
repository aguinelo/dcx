import { existsSync, realpathSync } from 'node:fs';
import path from 'node:path';

/**
 * Violations detected by the sandbox. Names mirror the FileStore codes observed
 * in the Claude Code binary (v2.1.308) so the two models stay comparable.
 */
export type SandboxViolation =
  | 'ESCAPES_ROOT'
  | 'IS_A_SYMLINK'
  | 'DOTDOT_NOT_PERMITTED'
  | 'ABSOLUTE_NOT_PERMITTED'
  | 'NOT_UTF8'
  | 'NOT_A_FILE'
  | 'NOT_A_DIRECTORY';

export class SandboxError extends Error {
  readonly code: SandboxViolation;

  constructor(code: SandboxViolation, message: string) {
    super(message);
    this.name = 'SandboxError';
    this.code = code;
  }
}

export interface SandboxOptions {
  /** Session working directory. Every relative path is resolved against it. */
  workdir: string;
  /**
   * Extra directories the file tools may reach, beyond `workdir`.
   * There is no `unrestrictedPaths`: confinement is by construction.
   */
  allowedRoots?: string[];
}

/**
 * True when `target` is `root` itself or lives beneath it.
 *
 * Uses `path.relative` rather than a string prefix so that `/tmp/rootevil`
 * is correctly rejected for root `/tmp/root`.
 */
export function isInside(root: string, target: string): boolean {
  const rel = path.relative(path.resolve(root), path.resolve(target));
  if (rel === '') return true;
  if (rel === '..') return false;
  if (rel.startsWith(`..${path.sep}`)) return false;
  if (path.isAbsolute(rel)) return false;
  return true;
}

/** Split on both POSIX and Windows separators so the check is platform-agnostic. */
function segments(input: string): string[] {
  return input.split(/[/\\]+/);
}

/**
 * Refuse `..` instead of sanitizing it away.
 *
 * Collapsing `a/../../etc/passwd` to `etc/passwd` would silently turn an
 * escape attempt into a valid-but-wrong path. Failing loudly keeps intent visible.
 */
export function assertNoDotDot(candidate: string): void {
  if (segments(candidate).includes('..')) {
    throw new SandboxError(
      'DOTDOT_NOT_PERMITTED',
      `".." is not permitted in the path: ${candidate}`,
    );
  }
}

/** Refuse absolute inputs where the contract is a workdir-relative path. */
export function assertNotAbsolute(candidate: string): void {
  if (path.isAbsolute(candidate) || /^[A-Za-z]:[/\\]/.test(candidate)) {
    throw new SandboxError(
      'ABSOLUTE_NOT_PERMITTED',
      `absolute path not permitted; pass a relative path: ${candidate}`,
    );
  }
}

/**
 * Real path of the nearest ancestor that actually exists.
 *
 * A path under `workdir` can still escape via a symlinked parent, so we resolve
 * what we can and confine the result.
 */
function realpathOfNearestExisting(target: string): string {
  let current = path.resolve(target);
  for (;;) {
    if (existsSync(current)) {
      try {
        return realpathSync(current);
      } catch {
        return current;
      }
    }
    const parent = path.dirname(current);
    if (parent === current) return current;
    current = parent;
  }
}

/**
 * Canonical form of a directory, when it exists.
 *
 * On macOS `tmpdir()` lives under `/var`, which is itself a symlink to
 * `/private/var`. Comparing a realpath against a non-canonical root would report
 * a false escape, so roots must be canonicalized before they are used as bounds.
 */
function canonicalize(target: string): string {
  const resolved = path.resolve(target);
  try {
    return realpathSync(resolved);
  } catch {
    return resolved;
  }
}

/** True when the candidate's real path stays inside at least one permitted root. */
function realPathIsContained(resolved: string, canonicalRoots: string[]): boolean {
  const real = realpathOfNearestExisting(resolved);
  return canonicalRoots.some((root) => isInside(root, real));
}

/** Refuse absolute inputs that do not land inside a permitted root. */
function assertAbsoluteIsPermitted(candidate: string, lexicalRoots: string[]): boolean {
  if (!path.isAbsolute(candidate) && !/^[A-Za-z]:[/\\]/.test(candidate)) return false;
  const resolved = path.resolve(candidate);
  if (lexicalRoots.some((root) => isInside(root, resolved))) return true;
  throw new SandboxError(
    'ABSOLUTE_NOT_PERMITTED',
    `absolute path not permitted (it is outside every permitted root): ${candidate}`,
  );
}

/**
 * Resolve a candidate path to an absolute one, refusing anything that leaves the
 * permitted roots.
 *
 * Rules:
 *   - `..` is always refused, never sanitized away
 *   - a relative path is resolved against the workdir
 *   - an absolute path is accepted only when it lands inside a permitted root;
 *     this is what makes `--add-dir` (allowedRoots) reachable
 *
 * Two containment checks, in order:
 *   1. lexical — cheap, catches the obvious cases
 *   2. real path — catches symlinked parents that survive a `..`-free traversal
 *
 * The returned path stays lexical so callers keep the user-facing shape;
 * only the containment bounds are canonicalized.
 */
export function resolveUnderRoot(candidate: string, options: SandboxOptions): string {
  const lexicalRoots = [
    path.resolve(options.workdir),
    ...(options.allowedRoots ?? []).map((r) => path.resolve(r)),
  ];
  const canonicalRoots = lexicalRoots.map(canonicalize);

  assertNoDotDot(candidate);

  const resolved = path.isAbsolute(candidate)
    ? path.resolve(candidate)
    : path.resolve(lexicalRoots[0] as string, candidate);

  if (!assertAbsoluteIsPermitted(candidate, lexicalRoots)) {
    assertNoDotDot(candidate);
  }

  const lexicallyContained = lexicalRoots.some((root) => isInside(root, resolved));
  if (!lexicallyContained) {
    throw new SandboxError(
      'ESCAPES_ROOT',
      `path escapes the permitted roots: ${candidate} -> ${resolved}`,
    );
  }

  if (!realPathIsContained(resolved, canonicalRoots)) {
    throw new SandboxError(
      'IS_A_SYMLINK',
      `path resolves outside the permitted roots via a symlink: ${candidate}`,
    );
  }

  return resolved;
}

/**
 * Validate a glob pattern.
 *
 * Patterns are refused rather than rewritten: an absolute pattern or a `..`
 * segment is a caller bug, and silently accepting it would hide intent.
 */
export function assertPatternLegal(pattern: string): void {
  assertNotAbsolute(pattern);
  assertNoDotDot(pattern);
  if (pattern.trim() === '') {
    throw new SandboxError('ABSOLUTE_NOT_PERMITTED', 'pattern is empty');
  }
}

/** Directories the sandbox currently permits. Useful for diagnostics and errors. */
export function permittedRoots(options: SandboxOptions): string[] {
  return [
    path.resolve(options.workdir),
    ...(options.allowedRoots ?? []).map((r) => path.resolve(r)),
  ];
}
