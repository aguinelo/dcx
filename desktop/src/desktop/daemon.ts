import path from 'node:path';

export interface HarnessCommand {
  command: string;
  args: string[];
}

export interface HarnessCommandOptions {
  packaged: boolean;
  resourcesPath: string;
  desktopRoot: string;
  platform?: NodeJS.Platform;
  env?: NodeJS.ProcessEnv;
}

/** Resolve the bundled CLI in production and the workspace build in development. */
export function resolveHarnessCommand(options: HarnessCommandOptions): HarnessCommand {
  const env = options.env ?? process.env;
  const executable = options.platform === 'win32' ? 'harness.exe' : 'harness';
  const override = env.HARNESS_BIN?.trim();
  if (override) return { command: override, args: ['serve', '--bind', '127.0.0.1:8787'] };

  if (options.packaged) {
    return {
      command: path.join(options.resourcesPath, 'bin', executable),
      args: ['serve', '--bind', '127.0.0.1:8787'],
    };
  }

  const repositoryRoot = path.resolve(options.desktopRoot, '..');
  return {
    command: path.join(repositoryRoot, 'target', 'debug', executable),
    args: ['serve', '--bind', '127.0.0.1:8787'],
  };
}

/** A custom endpoint belongs to the caller; only auto-manage the default local daemon. */
export function shouldManageHarness(env: NodeJS.ProcessEnv = process.env): boolean {
  if (env.HARNESS_MANAGE_DAEMON === '0') return false;
  const configuredUrl = env.HARNESS_URL;
  if (!configuredUrl) return true;
  try {
    const url = new URL(configuredUrl);
    return (
      ['127.0.0.1', 'localhost', '[::1]'].includes(url.hostname) &&
      (url.port === '' || url.port === '8787') &&
      (url.pathname === '' || url.pathname === '/')
    );
  } catch {
    return false;
  }
}
