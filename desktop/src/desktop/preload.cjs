/**
 * The only bridge between the renderer and the agent.
 *
 * The renderer gets a narrow, explicit surface: it can start a turn, cancel one,
 * read a context estimate, and receive events. It cannot reach the filesystem,
 * the shell, or `runAgent` itself.
 */
const { contextBridge, ipcRenderer } = require('electron');

/** @type {import('../agent/loop.ts').AgentEvent | { type: 'done' } | { type: 'error'; message: string }} */
let handler = () => {};

ipcRenderer.on('agent:event', (_event, payload) => handler(payload));

contextBridge.exposeInMainWorld('repl', {
  /** Start a turn. Resolves when the turn ends, however it ends. */
  run: (prompt, workspace, sessionId, contextWorkspaces) =>
    ipcRenderer.invoke('agent:run', prompt, workspace, sessionId, contextWorkspaces),

  /** Abort the turn in flight. */
  cancel: () => ipcRenderer.invoke('agent:cancel'),

  /** Context budget so the renderer can show the gauge. */
  context: (usedTokens) => ipcRenderer.invoke('session:context', usedTokens),

  /** Subscribe to agent events. Returns an unsubscribe function. */
  onEvent: (fn) => {
    handler = fn;
    return () => {
      handler = () => {};
    };
  },

  workspace: () => ipcRenderer.invoke('dcx:workspace'),
  pickWorkspace: () => ipcRenderer.invoke('dcx:pickWorkspace'),
  sessions: (workspace) => ipcRenderer.invoke('dcx:sessions', workspace),
  setPinned: (workspace, sessionId, pinned) =>
    ipcRenderer.invoke('dcx:setPinned', workspace, sessionId, pinned),
  session: (workspace, sessionId) => ipcRenderer.invoke('dcx:session', workspace, sessionId),
  projects: (root) => ipcRenderer.invoke('dcx:projects', root),
  config: (workspace) => ipcRenderer.invoke('dcx:config', workspace),
  setConfig: (workspace, updates) => ipcRenderer.invoke('dcx:setConfig', workspace, updates),
  account: (workspace) => ipcRenderer.invoke('dcx:account', workspace),
  gitBranch: (workspace) => ipcRenderer.invoke('dcx:gitBranch', workspace),
  githubRepos: () => ipcRenderer.invoke('dcx:githubRepos'),
  githubPrs: () => ipcRenderer.invoke('dcx:githubPrs'),
  runCatalog: (workspace) => ipcRenderer.invoke('dcx:runCatalog', workspace),
  loops: (workspace) => ipcRenderer.invoke('dcx:loops', workspace),
  pauseLoop: (workspace, id) => ipcRenderer.invoke('dcx:pauseLoop', workspace, id),
  resumeLoop: (workspace, id) => ipcRenderer.invoke('dcx:resumeLoop', workspace, id),
  artifact: (workspace, runId, name) => ipcRenderer.invoke('dcx:artifact', workspace, runId, name),
  searchSessions: (workspace, query) => ipcRenderer.invoke('dcx:searchSessions', workspace, query),
  routines: (workspace) => ipcRenderer.invoke('dcx:routines', workspace),
  saveRoutine: (workspace, routine) => ipcRenderer.invoke('dcx:saveRoutine', workspace, routine),
  deleteRoutine: (workspace, id) => ipcRenderer.invoke('dcx:deleteRoutine', workspace, id),
  addContextRepo: (workspace) => ipcRenderer.invoke('dcx:addContextRepo', workspace),
  authStatus: () => ipcRenderer.invoke('auth:status'),
  authLogout: () => ipcRenderer.invoke('auth:logout'),
  promote: (workspace, sessionId) => ipcRenderer.invoke('dcx:promote', workspace, sessionId),
  approve: (workspace, sessionId) => ipcRenderer.invoke('dcx:approve', workspace, sessionId),
  start: (workspace, sessionId) => ipcRenderer.invoke('dcx:start', workspace, sessionId),
  cancelRun: (workspace, runId) => ipcRenderer.invoke('dcx:cancelRun', workspace, runId),
  terminalRun: (workspace, command) => ipcRenderer.invoke('terminal:run', workspace, command),
  terminalRestart: (workspace) => ipcRenderer.invoke('terminal:restart', workspace),
});
