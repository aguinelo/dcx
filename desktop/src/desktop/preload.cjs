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
  run: (prompt) => ipcRenderer.invoke('agent:run', prompt),

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
});
