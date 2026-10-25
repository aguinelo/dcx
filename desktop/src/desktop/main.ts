import path from 'node:path';
import { fileURLToPath } from 'node:url';
import { BrowserWindow, Menu, app, ipcMain, shell } from 'electron';
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
let harnessSessionId: string | undefined;

function getHarnessWorkspace(): string {
  return process.env.HARNESS_WORKSPACE ?? app.getPath('home');
}

function getHarnessClient(): HarnessClient {
  return new HarnessClient({ baseUrl: process.env.HARNESS_URL ?? 'http://127.0.0.1:8787' });
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
  if (process.platform !== 'darwin') app.quit();
});

/**
 * One turn. The renderer never runs agent code — it only renders events,
 * so the sandbox and permission gate stay in the main process where the
 * filesystem and shell actually live.
 */
ipcMain.handle('agent:run', async (_event, prompt: string) => {
  const send = (payload: unknown): void => {
    if (mainWindow && !mainWindow.isDestroyed())
      mainWindow.webContents.send('agent:event', payload);
  };

  abortController = new AbortController();

  try {
    if (process.env.REPL_DESKTOP_LOCAL !== '1') {
      const client = getHarnessClient();
      const sessionId = await client.chat(
        getHarnessWorkspace(),
        prompt,
        harnessSessionId,
        (event) => {
          if (event.event === 'session' && typeof event.data.session_id === 'string') {
            harnessSessionId = event.data.session_id;
            return;
          }
          if (event.event === 'token' && typeof event.data.text === 'string') {
            send({ type: 'text', text: event.data.text });
          } else if (event.event === 'done') {
            send({ type: 'done' });
          } else if (event.event === 'error') {
            send({ type: 'error', message: String(event.data.message ?? 'erro do Harness') });
          }
        },
        abortController.signal,
      );
      harnessSessionId = sessionId ?? harnessSessionId;
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
ipcMain.handle('harness:session', (_event, workspace: string, sessionId: string) =>
  getHarnessClient().session(workspace, sessionId),
);
ipcMain.handle('harness:projects', (_event, root: string) => getHarnessClient().projects(root));
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
      mainWindow.webContents.send('agent:event', { type: 'run_event', event: event.event, data: event.data });
    }
  }).catch((error) => {
    if (mainWindow && !mainWindow.isDestroyed()) {
      mainWindow.webContents.send('agent:event', { type: 'error', message: error instanceof Error ? error.message : String(error) });
    }
  });
  return started;
});
