import path from 'node:path';

export interface DcxCommand {
  command: string;
  args: string[];
}

export interface DcxCommandOptions {
  packaged: boolean;
  resourcesPath: string;
  desktopRoot: string;
  platform?: NodeJS.Platform;
  env?: NodeJS.ProcessEnv;
}

const DAEMON_ARGS = ['--bind', '127.0.0.1:8787'];

/** Resolve the bundled `dcxd` daemon in production and the workspace build in development. */
export function resolveDcxCommand(options: DcxCommandOptions): DcxCommand {
  const env = options.env ?? process.env;
  const executable = options.platform === 'win32' ? 'dcxd.exe' : 'dcxd';
  const override = env.DCX_DAEMON_BIN?.trim();
  if (override) return { command: override, args: DAEMON_ARGS };

  if (options.packaged) {
    return { command: path.join(options.resourcesPath, 'bin', executable), args: DAEMON_ARGS };
  }

  const repositoryRoot = path.resolve(options.desktopRoot, '..');
  return { command: path.join(repositoryRoot, 'target', 'debug', executable), args: DAEMON_ARGS };
}

/** A custom endpoint belongs to the caller; only auto-manage the default local daemon. */
export function shouldManageDcx(env: NodeJS.ProcessEnv = process.env): boolean {
  if (env.DCX_MANAGE_DAEMON === '0') return false;
  const configuredUrl = env.DCX_URL;
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
