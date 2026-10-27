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

  workspace: () => ipcRenderer.invoke('harness:workspace'),
  sessions: (workspace) => ipcRenderer.invoke('harness:sessions', workspace),
  setPinned: (workspace, sessionId, pinned) =>
    ipcRenderer.invoke('harness:setPinned', workspace, sessionId, pinned),
  session: (workspace, sessionId) => ipcRenderer.invoke('harness:session', workspace, sessionId),
  projects: (root) => ipcRenderer.invoke('harness:projects', root),
  config: (workspace) => ipcRenderer.invoke('harness:config', workspace),
  setConfig: (workspace, updates) => ipcRenderer.invoke('harness:setConfig', workspace, updates),
  account: (workspace) => ipcRenderer.invoke('harness:account', workspace),
  gitBranch: (workspace) => ipcRenderer.invoke('harness:gitBranch', workspace),
  githubRepos: () => ipcRenderer.invoke('harness:githubRepos'),
  githubPrs: () => ipcRenderer.invoke('harness:githubPrs'),
  runCatalog: (workspace) => ipcRenderer.invoke('harness:runCatalog', workspace),
  artifact: (workspace, runId, name) => ipcRenderer.invoke('harness:artifact', workspace, runId, name),
  searchSessions: (workspace, query) => ipcRenderer.invoke('harness:searchSessions', workspace, query),
  routines: (workspace) => ipcRenderer.invoke('harness:routines', workspace),
  saveRoutine: (workspace, routine) => ipcRenderer.invoke('harness:saveRoutine', workspace, routine),
  deleteRoutine: (workspace, id) => ipcRenderer.invoke('harness:deleteRoutine', workspace, id),
  addContextRepo: (workspace) => ipcRenderer.invoke('harness:addContextRepo', workspace),
  authStatus: () => ipcRenderer.invoke('auth:status'),
  authLogout: () => ipcRenderer.invoke('auth:logout'),
  promote: (workspace, sessionId) => ipcRenderer.invoke('harness:promote', workspace, sessionId),
  approve: (workspace, sessionId) => ipcRenderer.invoke('harness:approve', workspace, sessionId),
  start: (workspace, sessionId) => ipcRenderer.invoke('harness:start', workspace, sessionId),
  cancelRun: (workspace, runId) => ipcRenderer.invoke('harness:cancelRun', workspace, runId),
  terminalRun: (workspace, command) => ipcRenderer.invoke('terminal:run', workspace, command),
  terminalRestart: (workspace) => ipcRenderer.invoke('terminal:restart', workspace),
});
