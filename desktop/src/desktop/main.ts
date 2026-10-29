import { type ChildProcess, execFile, spawn } from 'node:child_process';
import { access, realpath } from 'node:fs/promises';
import os from 'node:os';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { BrowserWindow, Menu, app, dialog, ipcMain, shell } from 'electron';
import { runAgent } from '../agent/loop.ts';
import { PermissionGate } from '../agent/permissions.ts';
import { DcxClient } from '../dcx/client.ts';
import { createAnthropicProvider } from '../provider/anthropic.ts';
import type { Provider } from '../provider/types.ts';
import { DEMO_SCRIPT, createScriptedProvider } from '../testing/scripted.ts';
import { ToolRegistry, defaultTools } from '../tools/registry.ts';
import { PersistentShell } from '../tools/shell.ts';
import type { ToolContext } from '../tools/types.ts';
import { resolveDcxCommand, shouldManageDcx } from './daemon.ts';

const here = path.dirname(fileURLToPath(import.meta.url));
const rendererRoot = path.join(here, 'renderer');

/** Rough token estimate; the provider reports exact usage per turn. */
const CONTEXT_LIMIT_TOKENS = 1_000_000;

let mainWindow: BrowserWindow | undefined;
let context: ToolContext | undefined;
let registry: ToolRegistry | undefined;
let abortController: AbortController | undefined;
const terminalShells = new Map<string, PersistentShell>();
let managedDcx: ChildProcess | undefined;

function getDcxWorkspace(): string {
  return process.env.DCX_WORKSPACE ?? app.getPath('home');
}

function getDcxClient(): DcxClient {
  return new DcxClient({ baseUrl: process.env.DCX_URL ?? 'http://127.0.0.1:8787' });
}

function delay(milliseconds: number): Promise<void> {
  return new Promise((resolve) => setTimeout(resolve, milliseconds));
}

async function startDcxIfNeeded(): Promise<void> {
  if (process.env.REPL_SCRIPTED === '1' || !shouldManageDcx()) return;
  try {
    await getDcxClient().health();
    return;
  } catch {
    // Start the bundled/local daemon below when the default endpoint is down.
  }

  const command = resolveDcxCommand({
    packaged: app.isPackaged,
    resourcesPath: process.resourcesPath,
    desktopRoot: path.resolve(here, '../..'),
  });
  try {
    await access(command.command);
  } catch {
    throw new Error(
      `Não encontrei o daemon dcxd em ${command.command}. Compile com cargo ou configure DCX_DAEMON_BIN.`,
    );
  }

  managedDcx = spawn(command.command, command.args, {
    cwd: getDcxWorkspace(),
    env: process.env,
    stdio: 'ignore',
    windowsHide: true,
  });
  managedDcx.on('error', (error) => {
    console.error(`Falha ao iniciar o daemon dcode: ${error.message}`);
  });

  for (let attempt = 0; attempt < 40; attempt += 1) {
    if (managedDcx.exitCode !== null) {
      throw new Error(
        `O daemon dcode encerrou durante a inicialização (código ${managedDcx.exitCode}).`,
      );
    }
    try {
      await getDcxClient().health();
      return;
    } catch {
      await delay(250);
    }
  }
  managedDcx.kill('SIGTERM');
  managedDcx = undefined;
  throw new Error('O daemon dcode não respondeu em até 10 segundos.');
}

/** Shells out to the same `claude` CLI the chat provider uses, so the account
 * menu reflects the real, shared Claude Code login rather than a local guess. */
function runClaudeAuth(args: string[]): Promise<string> {
  return new Promise((resolve, reject) => {
    execFile(process.env.DCX_CLAUDE_BIN ?? 'claude', ['auth', ...args], { timeout: 15_000 }, (error, stdout, stderr) => {
      if (error) {
        reject(new Error(stderr.trim() || error.message));
        return;
      }
      resolve(stdout);
    });
  });
}

function getContext(): ToolContext {
  context ??= {
    sandbox: { workdir: app.getPath('home') },
    shell: new PersistentShell(app.getPath('home')),
  };
  return context;
}

function getRegistry(): ToolRegistry {
  registry ??= new ToolRegistry(defaultTools());
  return registry;
}

function getProvider(): Provider {
  if (process.env.REPL_SCRIPTED === '1') return createScriptedProvider(DEMO_SCRIPT);
  return createAnthropicProvider({
    apiKey: process.env.ANTHROPIC_API_KEY ?? '',
    ...(process.env.ANTHROPIC_BASE_URL ? { baseUrl: process.env.ANTHROPIC_BASE_URL } : {}),
  });
}

function buildMenu(): void {
  const template: Electron.MenuItemConstructorOptions[] = [
    {
      label: 'Claude',
      submenu: [
        { role: 'about' },
        { type: 'separator' },
        { role: 'hide' },
        { role: 'hideOthers' },
        { role: 'unhide' },
        { type: 'separator' },
        { role: 'quit' },
      ],
    },
    {
      label: 'Arquivo',
      submenu: [{ role: 'close' }, { role: 'minimize' }, { role: 'zoom' }],
    },
    {
      label: 'Editar',
      submenu: [
        { role: 'undo' },
        { role: 'redo' },
        { type: 'separator' },
        { role: 'cut' },
        { role: 'copy' },
        { role: 'paste' },
        { role: 'selectAll' },
      ],
    },
    {
      label: 'Visualizar',
      submenu: [
        { role: 'resetZoom' },
        { role: 'zoomIn' },
        { role: 'zoomOut' },
        { type: 'separator' },
        { role: 'togglefullscreen' },
        { role: 'toggleDevTools' },
      ],
    },
    { label: 'Ir', submenu: [{ role: 'reload' }] },
    {
      label: 'Desenvolvedor',
      submenu: [{ role: 'toggleDevTools' }, { role: 'forceReload' }],
    },
    { label: 'Janela', submenu: [{ role: 'minimize' }, { role: 'zoom' }, { role: 'front' }] },
  ];

  Menu.setApplicationMenu(Menu.buildFromTemplate(template));
}

function createWindow(): void {
  mainWindow = new BrowserWindow({
    width: 1440,
    height: 900,
    minWidth: 960,
    minHeight: 600,
    show: false,
    backgroundColor: '#0d0d0f',
    titleBarStyle: 'hiddenInset',
    webPreferences: {
      preload: path.join(here, 'preload.cjs'),
      contextIsolation: true,
      nodeIntegration: false,
      sandbox: false,
    },
  });

  void mainWindow.loadFile(path.join(rendererRoot, 'index.html'));
  mainWindow.once('ready-to-show', () => mainWindow?.show());

  // External links open in the user's browser, never inside the shell.
  mainWindow.webContents.setWindowOpenHandler(({ url }) => {
    void shell.openExternal(url);
    return { action: 'deny' };
  });
}

app.whenReady().then(async () => {
  buildMenu();
  try {
    await startDcxIfNeeded();
  } catch (error) {
    console.error(error);
    await dialog.showMessageBox({
      type: 'warning',
      title: 'dcode indisponível',
      message: 'Não foi possível iniciar o daemon local do dcode.',
      detail: error instanceof Error ? error.message : String(error),
      buttons: ['Continuar'],
    });
  }
  createWindow();
  app.on('activate', () => {
    if (BrowserWindow.getAllWindows().length === 0) createWindow();
  });
});

app.on('window-all-closed', () => {
  getContext().shell.close();
  for (const shellSession of terminalShells.values()) shellSession.close();
  terminalShells.clear();
  if (process.platform !== 'darwin') app.quit();
});

app.on('before-quit', () => {
  managedDcx?.kill('SIGTERM');
  managedDcx = undefined;
});

/**
 * One turn. The renderer never runs agent code — it only renders events,
 * so the sandbox and permission gate stay in the main process where the
 * filesystem and shell actually live.
 */
ipcMain.handle('agent:run', async (_event, prompt: string, workspace: string | undefined, activeSessionId?: string, contextWorkspaces: string[] = []) => {
  const send = (payload: unknown): void => {
    if (mainWindow && !mainWindow.isDestroyed())
      mainWindow.webContents.send('agent:event', payload);
  };

  abortController = new AbortController();

  try {
    if (process.env.REPL_DESKTOP_LOCAL !== '1') {
      const client = getDcxClient();
      await client.chat(
        workspace || getDcxWorkspace(),
        prompt,
        activeSessionId,
        contextWorkspaces,
        (event) => {
          if (event.event === 'session' && typeof event.data.session_id === 'string') {
            send({ type: 'session', sessionId: event.data.session_id });
            return;
          }
          if (event.event === 'token' && typeof event.data.text === 'string') {
            send({ type: 'text', text: event.data.text });
          } else if (event.event === 'progress' && typeof event.data.message === 'string') {
            send({ type: 'progress', message: event.data.message });
          } else if (event.event === 'tool_start') {
            send({ type: 'tool_start', id: String(event.data.id ?? ''), name: String(event.data.name ?? 'tool'), input: event.data.input });
          } else if (event.event === 'tool_end') {
            send({ type: 'tool_end', id: String(event.data.id ?? ''), ok: event.data.ok === true, output: String(event.data.output ?? '') });
          } else if (event.event === 'done') {
            send({ type: 'done' });
          } else if (event.event === 'error') {
            send({ type: 'error', message: String(event.data.message ?? 'erro do dcode') });
          }
        },
        abortController.signal,
      );
      return;
    }
    const result = await runAgent({
      provider: getProvider(),
      registry: getRegistry(),
      gate: new PermissionGate({ mode: 'acceptEdits', ask: async () => true }),
      context: getContext(),
      systemPrompt: 'Você é o repl, um agente de programação em desktop.',
      model: process.env.REPL_MODEL ?? 'claude-sonnet-4-5',
      prompt,
      signal: abortController.signal,
      onEvent: (event) => send(event),
    });

    send({ type: 'done', ...result });
  } catch (error) {
    send({
      type: 'error',
      message: error instanceof Error ? error.message : String(error),
    });
  } finally {
    abortController = undefined;
  }
});

ipcMain.handle('dcx:addContextRepo', async (_event, workspace: string) => {
  const ownerWindow = mainWindow;
  if (!ownerWindow) return undefined;
  const picker = await dialog.showOpenDialog(ownerWindow, {
    title: 'Adicionar repositório ao contexto',
    buttonLabel: 'Selecionar repositório',
    properties: ['openDirectory'],
  });
  if (picker.canceled || !picker.filePaths[0]) return undefined;

  const selected = picker.filePaths[0];
  const gitRoot = (directory: string): Promise<string> => new Promise((resolve, reject) => {
    execFile('git', ['-C', directory, 'rev-parse', '--show-toplevel'], { timeout: 5_000 }, (error, stdout, stderr) => {
      if (error) reject(new Error(stderr.trim() || 'A pasta selecionada não é um repositório Git.'));
      else resolve(stdout.trim());
    });
  });
  const selectedRealPath = await realpath(selected);
  const canonical = await realpath(await gitRoot(selectedRealPath));
  const primary = await realpath(workspace);
  if (canonical === primary) throw new Error('Esse repositório já é o workspace principal.');

  let provider = 'modelo configurado';
  try {
    provider = (await getDcxClient().config(workspace)).provider;
  } catch {
    // Permission copy stays accurate even while the daemon is unavailable.
  }
  const answer = await dialog.showMessageBox(ownerWindow, {
    type: 'warning',
    title: 'Autorizar acesso ao repositório?',
    message: `Permitir que o dcode leia ${path.basename(canonical)}?`,
    detail: `Arquivos desse repositório poderão ser incluídos no contexto e enviados ao provider ${provider} durante as conversas deste workspace. Você pode remover o acesso pela barra lateral a qualquer momento.`,
    buttons: ['Cancelar', 'Permitir acesso'],
    defaultId: 0,
    cancelId: 0,
    noLink: true,
  });
  return answer.response === 1 ? canonical : undefined;
});

ipcMain.handle('terminal:run', async (_event, workspace: string, command: string) => {
  const trimmed = command.trim();
  if (!trimmed) throw new Error('Digite um comando.');
  if (trimmed.length > 4_000) throw new Error('O comando excede o limite de 4.000 caracteres.');
  const ownerWindow = mainWindow;
  if (!ownerWindow) throw new Error('A janela do desktop não está disponível.');
  const cwd = await realpath(workspace);
  const answer = await dialog.showMessageBox(ownerWindow, {
    type: 'warning',
    title: 'Executar comando no terminal?',
    message: `O comando será executado em ${cwd}`,
    detail: `${trimmed}\n\nComandos podem ler e alterar arquivos e acessar a rede. Execute apenas o que você revisou.`,
    buttons: ['Cancelar', 'Executar comando'],
    defaultId: 0,
    cancelId: 0,
    noLink: true,
  });
  if (answer.response !== 1) return { cancelled: true, stdout: '', exitCode: -1, timedOut: false };

  let shellSession = terminalShells.get(cwd);
  if (!shellSession) {
    shellSession = new PersistentShell(cwd);
    terminalShells.set(cwd, shellSession);
  }
  const result = await shellSession.run(trimmed, { timeoutMs: 60_000 });
  const outputLimit = 60_000;
  const truncated = result.stdout.length > outputLimit;
  return {
    cancelled: false,
    stdout: truncated ? `[saída truncada; exibindo os últimos ${outputLimit} caracteres]\n${result.stdout.slice(-outputLimit)}` : result.stdout,
    exitCode: result.exitCode,
    timedOut: result.timedOut,
  };
});

ipcMain.handle('terminal:restart', async (_event, workspace: string) => {
  const cwd = await realpath(workspace);
  terminalShells.get(cwd)?.restart();
  return true;
});

ipcMain.handle('agent:cancel', () => {
  abortController?.abort();
  return true;
});

ipcMain.handle('session:context', (_event, usedTokens: number) => {
  const used = Math.max(0, Math.min(usedTokens, CONTEXT_LIMIT_TOKENS));
  return {
    used,
    limit: CONTEXT_LIMIT_TOKENS,
    percent: Math.round((used / CONTEXT_LIMIT_TOKENS) * 100),
  };
});

ipcMain.handle('dcx:workspace', () => getDcxWorkspace());
ipcMain.handle('dcx:pickWorkspace', async () => {
  const ownerWindow = mainWindow;
  if (!ownerWindow) return undefined;
  const picker = await dialog.showOpenDialog(ownerWindow, {
    title: 'Abrir workspace',
    buttonLabel: 'Abrir workspace',
    properties: ['openDirectory'],
  });
  if (picker.canceled || !picker.filePaths[0]) return undefined;
  return realpath(picker.filePaths[0]);
});
ipcMain.handle('dcx:sessions', (_event, workspace: string) =>
  getDcxClient().sessions(workspace),
);
ipcMain.handle('dcx:setPinned', (_event, workspace: string, sessionId: string, pinned: boolean) =>
  getDcxClient().setPinned(workspace, sessionId, pinned),
);
ipcMain.handle('dcx:session', (_event, workspace: string, sessionId: string) =>
  getDcxClient().session(workspace, sessionId),
);
ipcMain.handle('dcx:projects', (_event, root: string) => getDcxClient().projects(root));
ipcMain.handle('dcx:config', (_event, workspace: string) => getDcxClient().config(workspace));
ipcMain.handle(
  'dcx:setConfig',
  (_event, workspace: string, updates: Parameters<DcxClient['setConfig']>[1]) =>
    getDcxClient().setConfig(workspace, updates),
);
ipcMain.handle('dcx:account', (_event, workspace: string) => getDcxClient().account(workspace));
ipcMain.handle('dcx:gitBranch', (_event, workspace: string) => getDcxClient().gitBranch(workspace));
ipcMain.handle('dcx:githubRepos', () => getDcxClient().githubRepos());
ipcMain.handle('dcx:githubPrs', () => getDcxClient().githubPrs());
ipcMain.handle('dcx:runCatalog', (_event, workspace: string) => getDcxClient().runCatalog(workspace));
ipcMain.handle('dcx:loops', (_event, workspace: string) => getDcxClient().loops(workspace));
ipcMain.handle('dcx:pauseLoop', (_event, workspace: string, id: string) => getDcxClient().pauseLoop(workspace, id));
ipcMain.handle('dcx:resumeLoop', (_event, workspace: string, id: string) => getDcxClient().resumeLoop(workspace, id));
ipcMain.handle('dcx:artifact', (_event, workspace: string, runId: string, name: string) => getDcxClient().artifact(workspace, runId, name));
ipcMain.handle('dcx:searchSessions', (_event, workspace: string, query: string) => getDcxClient().searchSessions(workspace, query));
ipcMain.handle('dcx:routines', (_event, workspace: string) => getDcxClient().routines(workspace));
ipcMain.handle('dcx:saveRoutine', (_event, workspace: string, routine: { id?: string; title: string; prompt: string; interval_hours: number | null }) => getDcxClient().saveRoutine(workspace, routine));
ipcMain.handle('dcx:deleteRoutine', (_event, workspace: string, id: string) => getDcxClient().deleteRoutine(workspace, id));
ipcMain.handle('dcx:compactSession', (_event, workspace: string, sessionId: string) => getDcxClient().compactSession(workspace, sessionId));

ipcMain.handle('env:pickFile', async () => {
  const result = await dialog.showOpenDialog({ properties: ['openFile', 'multiSelections'] });
  return result.canceled ? [] : result.filePaths;
});

ipcMain.handle('env:which', async (_event, cmd: string) => {
  const { execFile: execFileFn } = await import('node:child_process');
  const { promisify } = await import('node:util');
  try {
    const { stdout } = await promisify(execFileFn)('which', [cmd]);
    return { path: stdout.trim() };
  } catch {
    return { path: null };
  }
});

ipcMain.handle('env:openExternal', (_event, url: string) => {
  const allowed = ['https://cli.github.com', 'https://claude.ai', 'https://github.com/anthropics'];
  if (allowed.some((prefix) => url.startsWith(prefix))) return shell.openExternal(url);
});

ipcMain.handle('env:logPath', () => path.join(os.homedir(), '.dcx', 'daemon.log'));

ipcMain.handle('auth:status', async () => {
  try {
    const stdout = await runClaudeAuth(['status', '--json']);
    return JSON.parse(stdout) as {
      loggedIn: boolean;
      email?: string;
      orgName?: string;
      subscriptionType?: string;
    };
  } catch (error) {
    return { loggedIn: false, error: error instanceof Error ? error.message : String(error) };
  }
});
ipcMain.handle('auth:logout', async () => {
  try {
    await runClaudeAuth(['logout']);
  } catch {
    // claude CLI not installed — nothing to log out from
  }
  return { loggedIn: false };
});
ipcMain.handle('settings:setup', (_event, workspace: string) =>
  getDcxClient().setup(workspace),
);
ipcMain.handle(
  'settings:setCredentials',
  (_event, workspace: string, provider: string, apiKey: string) =>
    getDcxClient().setCredentials(workspace, provider as 'minimax' | 'claude', apiKey),
);
ipcMain.handle('dcx:promote', (_event, workspace: string, sessionId: string) =>
  getDcxClient().promote(workspace, sessionId),
);
ipcMain.handle('dcx:approve', (_event, workspace: string, sessionId: string) =>
  getDcxClient().approve(workspace, sessionId, true),
);
ipcMain.handle('dcx:start', async (_event, workspace: string, sessionId: string) => {
  const client = getDcxClient();
  const started = await client.start(workspace, sessionId);
  void client.events(workspace, started.run_id, (event) => {
    if (mainWindow && !mainWindow.isDestroyed()) {
      mainWindow.webContents.send('agent:event', { type: 'run_event', event: event.event, data: event.data, runId: started.run_id });
    }
  }).catch((error) => {
    if (mainWindow && !mainWindow.isDestroyed()) {
      mainWindow.webContents.send('agent:event', { type: 'error', message: error instanceof Error ? error.message : String(error) });
    }
  });
  return started;
});
ipcMain.handle('dcx:cancelRun', (_event, workspace: string, runId: string) => getDcxClient().cancel(workspace, runId));
