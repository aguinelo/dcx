import { execFile } from 'node:child_process';
import { realpath } from 'node:fs/promises';
import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { BrowserWindow, Menu, app, dialog, ipcMain, shell } from 'electron';
import { runAgent } from '../agent/loop.ts';
import { PermissionGate } from '../agent/permissions.ts';
import { createAnthropicProvider } from '../provider/anthropic.ts';
import type { Provider } from '../provider/types.ts';
import { DEMO_SCRIPT, createScriptedProvider } from '../testing/scripted.ts';
import { ToolRegistry, defaultTools } from '../tools/registry.ts';
import { PersistentShell } from '../tools/shell.ts';
import type { ToolContext } from '../tools/types.ts';
import { HarnessClient } from '../harness/client.ts';

const here = path.dirname(fileURLToPath(import.meta.url));
const rendererRoot = path.join(here, 'renderer');

/** Rough token estimate; the provider reports exact usage per turn. */
const CONTEXT_LIMIT_TOKENS = 1_000_000;

let mainWindow: BrowserWindow | undefined;
let context: ToolContext | undefined;
let registry: ToolRegistry | undefined;
let abortController: AbortController | undefined;
const terminalShells = new Map<string, PersistentShell>();

function getHarnessWorkspace(): string {
  return process.env.HARNESS_WORKSPACE ?? app.getPath('home');
}

function getHarnessClient(): HarnessClient {
  return new HarnessClient({ baseUrl: process.env.HARNESS_URL ?? 'http://127.0.0.1:8787' });
}

/** Shells out to the same `claude` CLI the chat provider uses, so the account
 * menu reflects the real, shared Claude Code login rather than a local guess. */
function runClaudeAuth(args: string[]): Promise<string> {
  return new Promise((resolve, reject) => {
    execFile(process.env.HARNESS_CLAUDE_BIN ?? 'claude', ['auth', ...args], { timeout: 15_000 }, (error, stdout, stderr) => {
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

app.whenReady().then(() => {
  buildMenu();
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
      const client = getHarnessClient();
      await client.chat(
        workspace || getHarnessWorkspace(),
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
            send({ type: 'error', message: String(event.data.message ?? 'erro do Harness') });
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

ipcMain.handle('harness:addContextRepo', async (_event, workspace: string) => {
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
    provider = (await getHarnessClient().config(workspace)).provider;
  } catch {
    // Permission copy stays accurate even while the daemon is unavailable.
  }
  const answer = await dialog.showMessageBox(ownerWindow, {
    type: 'warning',
    title: 'Autorizar acesso ao repositório?',
    message: `Permitir que o Harness leia ${path.basename(canonical)}?`,
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

ipcMain.handle('harness:workspace', () => getHarnessWorkspace());
ipcMain.handle('harness:sessions', (_event, workspace: string) =>
  getHarnessClient().sessions(workspace),
);
ipcMain.handle('harness:setPinned', (_event, workspace: string, sessionId: string, pinned: boolean) =>
  getHarnessClient().setPinned(workspace, sessionId, pinned),
);
ipcMain.handle('harness:session', (_event, workspace: string, sessionId: string) =>
  getHarnessClient().session(workspace, sessionId),
);
ipcMain.handle('harness:projects', (_event, root: string) => getHarnessClient().projects(root));
ipcMain.handle('harness:config', (_event, workspace: string) => getHarnessClient().config(workspace));
ipcMain.handle(
  'harness:setConfig',
  (_event, workspace: string, updates: Parameters<HarnessClient['setConfig']>[1]) =>
    getHarnessClient().setConfig(workspace, updates),
);
ipcMain.handle('harness:account', (_event, workspace: string) => getHarnessClient().account(workspace));
ipcMain.handle('harness:gitBranch', (_event, workspace: string) => getHarnessClient().gitBranch(workspace));
ipcMain.handle('harness:githubRepos', () => getHarnessClient().githubRepos());
ipcMain.handle('harness:githubPrs', () => getHarnessClient().githubPrs());
ipcMain.handle('harness:runCatalog', (_event, workspace: string) => getHarnessClient().runCatalog(workspace));
ipcMain.handle('harness:artifact', (_event, workspace: string, runId: string, name: string) => getHarnessClient().artifact(workspace, runId, name));
ipcMain.handle('harness:searchSessions', (_event, workspace: string, query: string) => getHarnessClient().searchSessions(workspace, query));

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
  await runClaudeAuth(['logout']);
  return { loggedIn: false };
});
ipcMain.handle('harness:promote', (_event, workspace: string, sessionId: string) =>
  getHarnessClient().promote(workspace, sessionId),
);
ipcMain.handle('harness:approve', (_event, workspace: string, sessionId: string) =>
  getHarnessClient().approve(workspace, sessionId, true),
);
ipcMain.handle('harness:start', async (_event, workspace: string, sessionId: string) => {
  const client = getHarnessClient();
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
ipcMain.handle('harness:cancelRun', (_event, workspace: string, runId: string) => getHarnessClient().cancel(workspace, runId));
