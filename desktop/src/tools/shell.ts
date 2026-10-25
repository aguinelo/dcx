import { type ChildProcessWithoutNullStreams, spawn } from 'node:child_process';
import path from 'node:path';

export interface ShellRunResult {
  stdout: string;
  exitCode: number;
  timedOut: boolean;
}

export interface ShellRunOptions {
  timeoutMs?: number;
}

interface Pending {
  resolve: (result: ShellRunResult) => void;
  timer: NodeJS.Timeout | undefined;
  out: string;
  /** Sentinel id used to frame this call's output. */
  key: string;
  /** Which spawned process owns this call. */
  generation: number;
}

const DEFAULT_TIMEOUT_MS = 120_000;
const SENTINEL = '__repl_done_';

/**
 * A real long-lived bash process.
 *
 * `cwd` and exported environment variables survive between calls, because the
 * process itself survives. Calls are serialized through a promise chain: two
 * concurrent `run`s on one shell would interleave their output in the same
 * stdout stream and could not be told apart.
 *
 * After `restart()` the previous process is still alive for a moment and will
 * emit `exit` late. Each pending call therefore records the generation that owns
 * it, so a dying process can never settle a call that belongs to its successor.
 */
export class PersistentShell {
  readonly workdir: string;

  private readonly env: NodeJS.ProcessEnv;
  private proc: ChildProcessWithoutNullStreams | undefined;
  private generation = 0;
  private readonly pending = new Set<Pending>();
  private queue: Promise<unknown> = Promise.resolve();
  private seq = 0;
  private closed = false;

  constructor(workdir: string, env: NodeJS.ProcessEnv = process.env) {
    this.workdir = path.resolve(workdir);
    this.env = { ...env };
  }

  private spawnProcess(): ChildProcessWithoutNullStreams {
    const proc = spawn('bash', ['--norc', '--noprofile'], {
      cwd: this.workdir,
      env: this.env,
      stdio: ['pipe', 'pipe', 'pipe'],
    });

    const generation = ++this.generation;

    proc.stdout.setEncoding('utf8');
    proc.stderr.setEncoding('utf8');

    const onData = (chunk: string): void => {
      for (const entry of this.pending) {
        if (entry.generation !== generation) continue;

        entry.out += chunk;
        const marker = `${SENTINEL}${entry.key}:`;
        const at = entry.out.indexOf(marker);
        if (at === -1) continue;

        const stdout = entry.out.slice(0, at);
        const rest = entry.out.slice(at + marker.length);
        const endOfCode = rest.search(/\D/);
        const code = Number.parseInt(endOfCode === -1 ? rest : rest.slice(0, endOfCode), 10);

        this.settle(entry, { stdout, exitCode: Number.isNaN(code) ? -1 : code, timedOut: false });
      }
    };

    proc.stdout.on('data', onData);
    // stderr is merged into the same stream so ordering is preserved.
    proc.stderr.on('data', onData);

    const onGone = (code: number | null): void => {
      if (this.proc === proc) this.proc = undefined;
      // Only calls owned by this process are settled here.
      for (const entry of [...this.pending]) {
        if (entry.generation !== generation) continue;
        this.settle(entry, { stdout: entry.out, exitCode: code ?? -1, timedOut: false });
      }
    };

    proc.on('exit', (code) => onGone(code));
    proc.on('error', () => onGone(null));

    return proc;
  }

  private settle(entry: Pending, result: ShellRunResult): void {
    if (entry.timer) clearTimeout(entry.timer);
    this.pending.delete(entry);
    entry.resolve(result);
  }

  private ensureProcess(): ChildProcessWithoutNullStreams {
    if (this.closed) throw new Error('shell is closed');
    this.proc ??= this.spawnProcess();
    return this.proc;
  }

  /** Queue an execution so the single shell never interleaves two commands. */
  run(command: string, options: ShellRunOptions = {}): Promise<ShellRunResult> {
    const scheduled = this.queue.then(
      () => this.exec(command, options),
      () => this.exec(command, options),
    );
    // Keep the chain alive even when a command fails.
    this.queue = scheduled.catch(() => undefined);
    return scheduled;
  }

  private exec(command: string, options: ShellRunOptions): Promise<ShellRunResult> {
    const proc = this.ensureProcess();
    const key = String(++this.seq);
    const timeoutMs = options.timeoutMs ?? DEFAULT_TIMEOUT_MS;
    // `ensureProcess` spawns and bumps the generation, so this is the owning one.
    const generation = this.generation;

    return new Promise<ShellRunResult>((resolve) => {
      const entry: Pending = {
        resolve,
        timer: undefined,
        out: '',
        generation,
        key,
      };
      entry.timer = setTimeout(() => {
        this.settle(entry, { stdout: entry.out, exitCode: -1, timedOut: true });
      }, timeoutMs);

      this.pending.add(entry);

      // Group so a compound command keeps a single exit status, and print the
      // sentinel on its own line so trailing output cannot corrupt the code.
      proc.stdin.write(
        `{\n${command}\n} 2>&1\nprintf '\\n%s%s:%s\\n' "${SENTINEL}" "${key}" "$?"\n`,
      );
    });
  }

  /**
   * Kill and respawn, preserving nothing.
   *
   * Mirrors the observed `bash session restarted`. Calls still queued against
   * the old process are failed rather than left hanging on a stream that will
   * never produce their sentinel again.
   */
  restart(): void {
    const dying = this.proc;
    this.proc = undefined;

    for (const entry of [...this.pending]) {
      if (dying && entry.generation === this.generation) {
        this.settle(entry, { stdout: entry.out, exitCode: -1, timedOut: false });
      }
    }

    dying?.kill('SIGKILL');
  }

  /**
   * Tear the shell down.
   *
   * SIGKILL rather than SIGTERM: the shell is disposable, and a non-interactive
   * bash blocked on `read` can defer SIGTERM until its current child exits,
   * which would keep a `sleep 30` alive and hold Node's event loop open.
   * `unref` guarantees the process can never block interpreter exit.
   */
  close(): void {
    this.closed = true;

    const proc = this.proc;
    this.proc = undefined;

    for (const entry of [...this.pending]) {
      this.settle(entry, { stdout: entry.out, exitCode: -1, timedOut: false });
    }

    if (!proc) return;

    proc.kill('SIGKILL');
    proc.stdin.destroy();
    proc.stdout.destroy();
    proc.stderr.destroy();
    proc.unref();
  }
}
