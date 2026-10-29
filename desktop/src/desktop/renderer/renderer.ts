/**
 * Renderer.
 *
 * Holds no agent logic. It renders events that arrive over the preload bridge
 * and sends prompts back. The only reason this file exists is the DOM.
 */

import type { DcxLoopSummary } from '../../dcx/client.ts';
import { describeLoop } from './loops.ts';
import { activityIndicatorMarkup, describeActivity, describeProgress, formatActivityElapsed } from './activity.ts';
import type { BackgroundTask } from './mock.ts';

interface AgentEventPayload {
  type: 'text' | 'progress' | 'tool_start' | 'tool_end' | 'denied' | 'done' | 'error' | 'run_event' | 'session';
  id?: string;
  name?: string;
  input?: unknown;
  output?: string;
  ok?: boolean;
  reason?: string;
  text?: string;
  message?: string;
  inputTokens?: number;
  outputTokens?: number;
  event?: string;
  data?: Record<string, unknown>;
  sessionId?: string;
  runId?: string;
}

interface ReplBridge {
  run(prompt: string, workspace: string, sessionId: string | undefined, contextWorkspaces: string[]): Promise<void>;
  cancel(): Promise<boolean>;
  context(usedTokens: number): Promise<{ used: number; limit: number; percent: number }>;
  onEvent(fn: (payload: AgentEventPayload) => void): () => void;
  workspace(): Promise<string>;
  pickWorkspace(): Promise<string | undefined>;
  sessions(
    workspace: string,
  ): Promise<Array<{ session_id: string; title: string; message_count: number; updated_at_unix_ms: number; pinned: boolean }>>;
  setPinned(workspace: string, sessionId: string, pinned: boolean): Promise<{ pinned: boolean }>;
  session(
    workspace: string,
    sessionId: string,
  ): Promise<{
    messages: Array<{
      role: 'user' | 'assistant';
      content: string;
      cost_usd?: number;
      input_tokens?: number;
      output_tokens?: number;
      cache_creation_input_tokens?: number;
      cache_read_input_tokens?: number;
    }>;
    interrupted_calls?: Array<{
      input_tokens?: number;
      output_tokens?: number;
      cache_creation_input_tokens?: number;
      cache_read_input_tokens?: number;
      cost_complete: boolean;
    }>;
  }>;
  projects(root: string): Promise<Array<{ path: string }>>;
  promote(workspace: string, sessionId: string): Promise<{ status: string; request: string }>;
  approve(workspace: string, sessionId: string): Promise<unknown>;
  start(workspace: string, sessionId: string): Promise<{ run_id: string; status: string }>;
  cancelRun(workspace: string, runId: string): Promise<void>;
  loops(workspace: string): Promise<{ loops: DcxLoopSummary[] }>;
  pauseLoop(workspace: string, id: string): Promise<void>;
  resumeLoop(workspace: string, id: string): Promise<void>;
  runCatalog(workspace: string): Promise<{ runs: Array<{ run_id: string; title: string; summary: string; status: string; criteria_passed: number | null; criteria_total: number | null; states: string[]; files: string[] }> }>;
  artifact(workspace: string, runId: string, name: string): Promise<{ name: string; content: string }>;
  searchSessions(workspace: string, query: string): Promise<{ results: Array<{ session_id: string; title: string; snippet: string; role: string; updated_at_unix_ms: number }> }>;
  routines(workspace: string): Promise<{ routines: Array<{ id: string; title: string; prompt: string; created_at_unix_ms: number; updated_at_unix_ms: number; interval_hours: number | null; next_run_at_unix_ms: number | null; last_run_at_unix_ms: number | null; last_run_status: string | null; last_run_error: string | null; last_cost_usd: number | null }> }>;
  saveRoutine(workspace: string, routine: { id?: string; title: string; prompt: string; interval_hours: number | null }): Promise<{ id: string; title: string; prompt: string; created_at_unix_ms: number; updated_at_unix_ms: number; interval_hours: number | null; next_run_at_unix_ms: number | null; last_run_at_unix_ms: number | null; last_run_status: string | null; last_run_error: string | null; last_cost_usd: number | null }>;
  deleteRoutine(workspace: string, id: string): Promise<{ id: string; deleted: boolean }>;
  config(workspace: string): Promise<{ provider: string; model?: string; web_enabled: boolean; max_tool_calls: number }>;
  setConfig(
    workspace: string,
    updates: Partial<{ provider: string; web_enabled: boolean; max_tool_calls: number }>,
  ): Promise<{ provider: string; model?: string; web_enabled: boolean; max_tool_calls: number }>;
  account(workspace: string): Promise<{ name: string; email?: string }>;
  gitBranch(workspace: string): Promise<{ branch: string | null }>;
  githubRepos(): Promise<{ repos: string[] }>;
  githubPrs(): Promise<{
    prs: Array<{ title: string; repo: string; number: number; url: string; additions: number; deletions: number; files: number; ready: boolean }>;
  }>;
  addContextRepo(workspace: string): Promise<string | undefined>;
  authStatus(): Promise<{ loggedIn: boolean; email?: string; orgName?: string; subscriptionType?: string; error?: string }>;
  authLogout(): Promise<{ loggedIn: boolean }>;
  setup(workspace: string): Promise<{ provider: string | null; has_minimax_key: boolean; has_anthropic_key: boolean; has_claude_cli: boolean }>;
  setCredentials(workspace: string, provider: 'minimax' | 'claude', apiKey: string): Promise<{ provider: string; model?: string; web_enabled: boolean; max_tool_calls: number }>;
  terminalRun(workspace: string, command: string): Promise<{ cancelled: boolean; stdout: string; exitCode: number; timedOut: boolean }>;
  terminalRestart(workspace: string): Promise<boolean>;
  compactSession(workspace: string, sessionId: string): Promise<{ kept: number; removed: number }>;
  pickFile(): Promise<string[]>;
  which(cmd: string): Promise<{ path: string | null }>;
  openExternal(url: string): Promise<void>;
  logPath(): Promise<string>;
}

const bridge = (window as unknown as { repl: ReplBridge }).repl;

const el = <T extends HTMLElement>(id: string): T => {
  const found = document.getElementById(id);
  if (!found) throw new Error(`elemento ausente: ${id}`);
  return found as T;
};

const thread = el<HTMLDivElement>('thread-inner');
const input = el<HTMLTextAreaElement>('input');
const layout = el<HTMLDivElement>('layout');
const ctxFill = el<HTMLDivElement>('ctx-fill');
const ctxPct = el<HTMLSpanElement>('ctx-pct');
const ctxCount = el<HTMLSpanElement>('ctx-count');

let busy = false;
let currentWorkspace = '';
let currentSessionId: string | undefined;
let streamingAssistantBody: HTMLElement | undefined;
let streamingAssistantText = '';
let streamingPendingText = '';
let streamFlushTimer: number | undefined;
let thinkingNode: HTMLElement | undefined;
let thinkingBody: HTMLElement | undefined;
let activityStartedAt = 0;
let activityTimer: number | undefined;
let currentProjects: Array<{ path: string }> = [];
let terminalWorkspaceDisplayed = '';
let selectedArtifactRun: string | undefined;
let searchSequence = 0;
let currentRoutineId: string | undefined;
let currentRoutineIntervalHours: number | null = null;

const CONTEXT_REPOS_KEY = 'dcx.contextRepos';
const WORKSPACES_KEY = 'dcx.workspaces';

const sessionHistory: string[] = [];
let sessionHistoryIndex = -1;
let attachedFiles: string[] = [];

function savedWorkspaces(): string[] {
  try {
    const value = JSON.parse(localStorage.getItem(WORKSPACES_KEY) ?? '[]') as unknown;
    return Array.isArray(value) ? value.filter((entry): entry is string => typeof entry === 'string') : [];
  } catch {
    return [];
  }
}

function saveWorkspacePath(workspace: string): void {
  const workspaces = savedWorkspaces().filter((entry) => entry !== workspace);
  workspaces.unshift(workspace);
  localStorage.setItem(WORKSPACES_KEY, JSON.stringify(workspaces.slice(0, 30)));
}

function contextRepos(): Record<string, string[]> {
  try {
    const value = JSON.parse(localStorage.getItem(CONTEXT_REPOS_KEY) ?? '{}') as unknown;
    return value && typeof value === 'object' && !Array.isArray(value)
      ? value as Record<string, string[]>
      : {};
  } catch {
    return {};
  }
}

function reposForWorkspace(workspace = currentWorkspace): string[] {
  return (contextRepos()[workspace] ?? []).filter((repo) => typeof repo === 'string');
}

function saveRepos(workspace: string, repos: string[]): void {
  const all = contextRepos();
  all[workspace] = repos;
  localStorage.setItem(CONTEXT_REPOS_KEY, JSON.stringify(all));
}

function renderContextRepos(): HTMLElement {
  const section = document.createElement('section');
  section.className = 'group context-repos';
  const title = document.createElement('h2');
  title.className = 'group__title';
  const label = document.createElement('span');
  label.textContent = 'Repositórios no contexto';
  const add = document.createElement('button');
  add.className = 'group__add';
  add.textContent = '+';
  add.dataset.action = 'add-context-repo';
  add.title = 'Adicionar repositório externo com confirmação';
  add.setAttribute('aria-label', add.title);
  title.append(label, add);

  const list = document.createElement('ul');
  list.className = 'conversations';
  for (const repo of reposForWorkspace()) {
    const item = document.createElement('li');
    item.className = 'context-repo';
    const name = document.createElement('span');
    name.className = 'title';
    name.textContent = repo.replace(/\/+$/, '').split('/').pop() || repo;
    name.title = repo;
    const remove = document.createElement('button');
    remove.className = 'context-repo__remove';
    remove.dataset.action = 'remove-context-repo';
    remove.dataset.repoPath = repo;
    remove.title = 'Remover acesso';
    remove.setAttribute('aria-label', `Remover acesso a ${name.textContent}`);
    remove.textContent = '×';
    item.append(name, remove);
    list.append(item);
  }
  if (list.childElementCount === 0) {
    const empty = document.createElement('li');
    empty.className = 'context-repo__empty';
    empty.textContent = 'Nenhum repositório compartilhado';
    list.append(empty);
  }
  section.append(title, list);
  return section;
}

async function addContextRepo(): Promise<void> {
  try {
    const selected = await bridge.addContextRepo(currentWorkspace);
    if (!selected) return;
    const repos = reposForWorkspace();
    if (!repos.includes(selected)) saveRepos(currentWorkspace, [...repos, selected]);
    renderWorkspaceGroups(currentProjects);
    showNotice('Acesso concedido', `${selected} poderá ser consultado nas próximas mensagens deste workspace.`, false);
  } catch (error) {
    showNotice('Não foi possível adicionar o repositório', error instanceof Error ? error.message : String(error), true);
  }
}

function renderWorkspaceGroups(projects: Array<{ path: string }> = []): void {
  const recent = savedWorkspaces().map((path) => ({ path }));
  const combined = new Map([...projects, ...recent].map((project) => [project.path, project]));
  el('groups').replaceChildren(
    renderContextRepos(),
    projectSection({
      name: 'Workspaces',
      items: [...combined.values()].map((project) => ({
        title: project.path.replace(/\/+$/, '').split('/').pop() || project.path,
        workspacePath: project.path,
        active: project.path === currentWorkspace,
      })),
    }),
  );
}

async function openArtifacts(): Promise<void> {
  el<HTMLElement>('search-view').hidden = true;
  el<HTMLElement>('routines-view').hidden = true;
  el<HTMLElement>('artifacts-view').hidden = false;
  await loadArtifactCatalog();
}

function openSearch(): void {
  el<HTMLElement>('artifacts-view').hidden = true;
  el<HTMLElement>('routines-view').hidden = true;
  el<HTMLElement>('search-view').hidden = false;
  const input = el<HTMLInputElement>('session-search-input');
  input.focus();
  input.select();
}

async function searchSessions(query: string): Promise<void> {
  const sequence = ++searchSequence;
  const list = el<HTMLUListElement>('session-search-results');
  list.replaceChildren();
  if (query.trim().length < 2) {
    const empty = document.createElement('li');
    empty.className = 'artifacts-view__empty';
    empty.textContent = 'Digite pelo menos 2 caracteres.';
    list.append(empty);
    return;
  }
  const loading = document.createElement('li');
  loading.className = 'artifacts-view__empty';
  loading.textContent = 'Buscando nas conversas…';
  list.append(loading);
  try {
    const { results } = await bridge.searchSessions(currentWorkspace, query.trim());
    if (sequence !== searchSequence) return;
    list.replaceChildren();
    for (const result of results) {
      const item = document.createElement('li');
      const button = document.createElement('button');
      button.className = 'session-search-result';
      button.dataset.action = 'open-search-result';
      button.dataset.sessionId = result.session_id;
      const title = document.createElement('strong');
      title.textContent = result.title || 'Nova conversa';
      const snippet = document.createElement('span');
      snippet.textContent = result.snippet;
      const meta = document.createElement('small');
      meta.textContent = result.role === 'user' ? 'Você' : 'Assistente';
      button.append(title, snippet, meta);
      item.append(button);
      list.append(item);
    }
    if (results.length === 0) {
      const empty = document.createElement('li');
      empty.className = 'artifacts-view__empty';
      empty.textContent = 'Nenhuma mensagem encontrada neste workspace.';
      list.append(empty);
    }
  } catch (error) {
    if (sequence !== searchSequence) return;
    const empty = document.createElement('li');
    empty.className = 'artifacts-view__empty';
    empty.textContent = error instanceof Error ? error.message : String(error);
    list.replaceChildren(empty);
  }
}

async function loadRoutines(selectedId = currentRoutineId, syncEditor = true): Promise<void> {
  const list = el<HTMLUListElement>('routine-list');
  list.replaceChildren();
  try {
    const { routines } = await bridge.routines(currentWorkspace);
    if (routines.length === 0) {
      const empty = document.createElement('li');
      empty.className = 'artifacts-view__empty';
      empty.textContent = 'Nenhuma rotina salva neste workspace.';
      list.append(empty);
      currentRoutineId = undefined;
      return;
    }
    for (const routine of routines) {
      const item = document.createElement('li');
      const button = document.createElement('button');
      button.className = `routine-item${routine.id === selectedId ? ' is-active' : ''}`;
      button.dataset.action = 'select-routine';
      button.dataset.routineId = routine.id;
      const title = document.createElement('strong');
      title.textContent = routine.title;
      const excerpt = document.createElement('span');
      excerpt.textContent = routine.prompt;
      button.append(title, excerpt);
      if (routine.interval_hours !== null) {
        const schedule = document.createElement('span');
        const status = routine.last_run_status ? ` · última: ${routine.last_run_status}` : '';
        const next = ` · próxima: ${formatRoutineTime(routine.next_run_at_unix_ms)}`;
        const cost = routine.last_cost_usd !== null ? ` · US$ ${routine.last_cost_usd.toFixed(4)}` : '';
        schedule.textContent = `A cada ${routine.interval_hours} h${status}${cost}${next}`;
        button.append(schedule);
        if (routine.last_run_error) {
          const error = document.createElement('span');
          error.textContent = routine.last_run_error;
          button.append(error);
        }
      }
      item.append(button);
      list.append(item);
    }
    if (syncEditor) {
      const routine = routines.find((item) => item.id === selectedId) ?? routines[0]!;
      setRoutineEditor(routine);
    }
  } catch (error) {
    const empty = document.createElement('li');
    empty.className = 'artifacts-view__empty';
    empty.textContent = error instanceof Error ? error.message : String(error);
    list.append(empty);
  }
}

function formatRoutineTime(timestamp: number | null): string {
  return timestamp === null ? '—' : new Date(timestamp).toLocaleString(undefined, { dateStyle: 'short', timeStyle: 'short' });
}

function setRoutineEditor(routine?: { id: string; title: string; prompt: string; interval_hours?: number | null }): void {
  currentRoutineId = routine?.id;
  currentRoutineIntervalHours = routine?.interval_hours ?? null;
  el<HTMLInputElement>('routine-id').value = routine?.id ?? '';
  el<HTMLInputElement>('routine-title').value = routine?.title ?? '';
  el<HTMLTextAreaElement>('routine-prompt').value = routine?.prompt ?? '';
  el<HTMLSelectElement>('routine-interval').value = currentRoutineIntervalHours?.toString() ?? '';
  el<HTMLFormElement>('routine-editor').querySelector<HTMLButtonElement>('[data-action="delete-routine"]')!.hidden = !routine;
  el<HTMLElement>('routine-feedback').hidden = true;
  el<HTMLElement>('routine-feedback').textContent = '';
  document.querySelectorAll<HTMLElement>('.routine-item').forEach((node) => {
    node.classList.toggle('is-active', node.dataset.routineId === routine?.id);
  });
}

function openRoutines(): void {
  el<HTMLDivElement>('notice').hidden = true;
  el<HTMLElement>('search-view').hidden = true;
  el<HTMLElement>('artifacts-view').hidden = true;
  el<HTMLElement>('routines-view').hidden = false;
  void loadRoutines();
}

async function saveCurrentRoutine(event: SubmitEvent): Promise<void> {
  event.preventDefault();
  const title = el<HTMLInputElement>('routine-title').value.trim();
  const prompt = el<HTMLTextAreaElement>('routine-prompt').value.trim();
  const intervalValue = el<HTMLSelectElement>('routine-interval').value;
  const intervalHours = intervalValue === '' ? null : Number.parseInt(intervalValue, 10);
  if (!title || !prompt) return;
  if (intervalHours !== null && intervalHours !== currentRoutineIntervalHours) {
    const accepted = window.confirm(
      `Agendar “${title}” a cada ${intervalHours} hora(s)? O dcode enviará as instruções ao modelo automaticamente, o que pode gerar custos. O daemon local precisa estar aberto.`,
    );
    if (!accepted) return;
  }
  try {
    const saved = await bridge.saveRoutine(currentWorkspace, {
      ...(currentRoutineId ? { id: currentRoutineId } : {}),
      title,
      prompt,
      interval_hours: intervalHours,
    });
    currentRoutineId = saved.id;
    await loadRoutines(saved.id);
    el<HTMLElement>('routine-feedback').textContent = `“${saved.title}” foi salva neste workspace.`;
    el<HTMLElement>('routine-feedback').hidden = false;
  } catch (error) {
    el<HTMLElement>('routine-feedback').textContent = error instanceof Error ? error.message : String(error);
    el<HTMLElement>('routine-feedback').hidden = false;
  }
}

async function deleteCurrentRoutine(): Promise<void> {
  if (!currentRoutineId) return;
  const title = el<HTMLInputElement>('routine-title').value || 'esta rotina';
  if (!window.confirm(`Remover “${title}” deste workspace?`)) return;
  try {
    await bridge.deleteRoutine(currentWorkspace, currentRoutineId);
    currentRoutineId = undefined;
    setRoutineEditor();
    await loadRoutines();
  } catch (error) {
    showNotice('Não foi possível remover a rotina', error instanceof Error ? error.message : String(error), true);
  }
}

function useCurrentRoutine(): void {
  const prompt = el<HTMLTextAreaElement>('routine-prompt').value.trim();
  if (!prompt) return;
  thread.replaceChildren();
  currentSessionId = undefined;
  accumulatedTokens = 0;
  lastUsageTotals = { messages: 0, cacheCreate: 0, output: 0 };
  void updateContext(0);
  el<HTMLButtonElement>('promote-run').hidden = true;
  document.querySelectorAll('[data-session-id].is-active').forEach((node) => node.classList.remove('is-active'));
  el<HTMLElement>('routines-view').hidden = true;
  input.value = prompt;
  autoGrow();
  input.focus();
}

async function loadArtifactCatalog(): Promise<void> {
  const list = el<HTMLUListElement>('artifact-runs');
  list.replaceChildren();
  el('artifact-files').replaceChildren();
  el('artifact-preview').textContent = 'Selecione um arquivo JSON para visualizar seu conteúdo.';
  try {
    const { runs } = await bridge.runCatalog(currentWorkspace);
    if (runs.length === 0) {
      const empty = document.createElement('li');
      empty.className = 'artifacts-view__empty';
      empty.textContent = 'Este workspace ainda não tem runs do dcode.';
      list.append(empty);
      el('artifact-run-title').textContent = 'Nenhum run encontrado';
      selectedArtifactRun = undefined;
      return;
    }
    for (const run of runs) {
      const item = document.createElement('li');
      const button = document.createElement('button');
      button.className = `artifact-run${run.run_id === selectedArtifactRun ? ' is-active' : ''}`;
      button.dataset.action = 'select-artifact-run';
      button.dataset.runId = run.run_id;
      const title = document.createElement('span');
      title.className = 'artifact-run__title';
      title.textContent = run.title || run.run_id;
      const meta = document.createElement('span');
      meta.className = 'artifact-run__meta';
      const criteria = run.criteria_passed !== null && run.criteria_total !== null
        ? ` · ${run.criteria_passed}/${run.criteria_total} critérios`
        : '';
      meta.textContent = `${run.status} · ${run.files.length} arquivos${criteria}`;
      button.append(title, meta);
      item.append(button);
      list.append(item);
    }
    const active = runs.find((run) => run.run_id === selectedArtifactRun) ?? runs[0]!;
    await selectArtifactRun(active.run_id, active.title, active.files);
  } catch (error) {
    const empty = document.createElement('li');
    empty.className = 'artifacts-view__empty';
    empty.textContent = error instanceof Error ? error.message : String(error);
    list.append(empty);
  }
}

async function selectArtifactRun(runId: string, title: string, files?: string[]): Promise<void> {
  selectedArtifactRun = runId;
  document.querySelectorAll<HTMLElement>('.artifact-run').forEach((node) => {
    node.classList.toggle('is-active', node.dataset.runId === runId);
  });
  el('artifact-run-title').textContent = title || runId;
  el('artifact-preview').textContent = 'Selecione um arquivo JSON para visualizar seu conteúdo.';
  const names = files ?? (await bridge.runCatalog(currentWorkspace)).runs.find((run) => run.run_id === runId)?.files ?? [];
  const list = el<HTMLUListElement>('artifact-files');
  list.replaceChildren();
  for (const name of names) {
    const item = document.createElement('li');
    const button = document.createElement('button');
    button.className = 'artifact-file';
    button.dataset.action = 'open-artifact-file';
    button.dataset.runId = runId;
    button.dataset.artifactName = name;
    button.textContent = name;
    item.append(button);
    list.append(item);
  }
  if (names.length === 0) {
    const empty = document.createElement('li');
    empty.className = 'artifacts-view__empty';
    empty.textContent = 'Este run ainda não gerou arquivos JSON.';
    list.append(empty);
  }
}

async function openArtifactFile(runId: string, name: string): Promise<void> {
  const preview = el<HTMLElement>('artifact-preview');
  preview.textContent = 'Carregando artefato…';
  try {
    const artifact = await bridge.artifact(currentWorkspace, runId, name);
    try {
      preview.textContent = JSON.stringify(JSON.parse(artifact.content), null, 2);
    } catch {
      preview.textContent = artifact.content;
    }
    document.querySelectorAll<HTMLElement>('.artifact-file').forEach((node) => {
      node.classList.toggle('is-active', node.dataset.artifactName === name && node.dataset.runId === runId);
    });
  } catch (error) {
    preview.textContent = error instanceof Error ? error.message : String(error);
  }
}

/* ── Sidebar and panel content ────────────────────────────────── */

function conversationNode(item: { title: string; active?: boolean; sessionId?: string; pinned?: boolean; workspacePath?: string }): HTMLElement {
  const node = document.createElement('li');
  if (item.active) node.classList.add('is-active');

  const glyph = document.createElement('span');
  glyph.className = 'glyph';
  glyph.textContent = '</>';

  const title = document.createElement('span');
  title.className = 'title';
  title.textContent = item.title;

  node.append(glyph, title);

  if (item.workspacePath) {
    node.dataset.workspacePath = item.workspacePath;
    node.title = item.workspacePath;
  }

  if (item.sessionId) {
    node.dataset.sessionId = item.sessionId;
    const pin = document.createElement('button');
    pin.className = 'conv-pin';
    pin.dataset.action = 'toggle-pin';
    pin.dataset.pinned = String(item.pinned ?? false);
    pin.title = item.pinned ? 'Desafixar' : 'Fixar';
    pin.textContent = item.pinned ? '★' : '☆';
    node.append(pin);
  }

  return node;
}

function renderAllConversations(items: Array<{ title: string; active?: boolean; sessionId?: string; pinned?: boolean }>): void {
  el<HTMLUListElement>('all-conversations').replaceChildren(...items.map(conversationNode));
}

function renderPinned(items: Array<{ title: string; active?: boolean; sessionId?: string; pinned?: boolean }>): void {
  el<HTMLUListElement>('pinned').replaceChildren(...items.map(conversationNode));
}

async function togglePin(sessionId: string, pinned: boolean): Promise<void> {
  try {
    await bridge.setPinned(currentWorkspace, sessionId, pinned);
    const sessions = await bridge.sessions(currentWorkspace);
    renderSessionLists(sessions);
  } catch (error) {
    showNotice('Não foi possível fixar', error instanceof Error ? error.message : String(error), true);
  }
}

function renderSessionLists(
  sessions: Array<{ session_id: string; title: string; pinned: boolean }>,
): void {
  renderPinned(
    sessions
      .filter((session) => session.pinned)
      .map((session) => ({
        title: session.title || 'Nova conversa',
        sessionId: session.session_id,
        pinned: true,
        active: session.session_id === currentSessionId,
      })),
  );
  renderAllConversations(
    sessions
      .filter((session) => !session.pinned)
      .map((session) => ({
        title: session.title || 'Nova conversa',
        sessionId: session.session_id,
        pinned: false,
        active: session.session_id === currentSessionId,
      })),
  );
}

function pushSessionHistory(sessionId: string): void {
  if (sessionHistory[sessionHistoryIndex] === sessionId) return;
  sessionHistory.splice(sessionHistoryIndex + 1);
  sessionHistory.push(sessionId);
  sessionHistoryIndex = sessionHistory.length - 1;
}

async function loadSession(workspace: string, sessionId: string): Promise<void> {
  try {
    const session = await bridge.session(workspace, sessionId);
    currentWorkspace = workspace;
    currentSessionId = sessionId;
    pushSessionHistory(sessionId);
    el<HTMLButtonElement>('promote-run').hidden = false;
    clearThinking();
    thread.replaceChildren();
    streamingAssistantBody = undefined;
    streamingAssistantText = '';
    streamingPendingText = '';
    for (const message of session.messages) {
      if (message.role === 'user') addUserMessage(message.content);
      else addAssistantMessage(message.content, false);
    }
    document.querySelectorAll('[data-session-id]').forEach((node) => {
      node.classList.toggle('is-active', (node as HTMLElement).dataset.sessionId === sessionId);
    });
    lastUsageTotals = sumUsage([...session.messages, ...(session.interrupted_calls ?? [])]);
    accumulatedTokens = lastUsageTotals.messages + lastUsageTotals.cacheCreate + lastUsageTotals.output;
    void updateContext(accumulatedTokens);
  } catch (error) {
    showNotice('Falha ao carregar sessão', error instanceof Error ? error.message : String(error), true);
  }
}

function projectSection(group: { name: string; items: Array<{ title: string; active?: boolean; workspacePath?: string }> }): HTMLElement {
  const section = document.createElement('section');
  section.className = 'group';

  const title = document.createElement('h2');
  title.className = 'group__title';

  const label = document.createElement('span');
  label.textContent = group.name;

  const add = document.createElement('button');
  add.className = 'group__add';
  add.textContent = '+';
  add.dataset.action = 'add-workspace';
  add.setAttribute('aria-label', 'Adicionar workspace');

  title.append(label, add);

  const list = document.createElement('ul');
  list.className = 'conversations';
  list.replaceChildren(...group.items.map(conversationNode));

  section.append(title, list);
  return section;
}

async function switchWorkspace(workspace: string): Promise<void> {
  if (!workspace || workspace === currentWorkspace || busy) return;
  currentWorkspace = workspace;
  currentSessionId = undefined;
  clearThinking();
  thread.replaceChildren();
  streamingAssistantBody = undefined;
  streamingAssistantText = '';
  streamingPendingText = '';
  accumulatedTokens = 0;
  lastUsageTotals = { messages: 0, cacheCreate: 0, output: 0 };
  el<HTMLButtonElement>('promote-run').hidden = true;
  void updateContext(0);
  try {
    localStorage.setItem('dcx.workspace', workspace);
    saveWorkspacePath(workspace);
  } catch {
    // Keep the selection for this window when storage is unavailable.
  }
  await loadDcxData(workspace);
}

function envNode(glyph: string, label: string, config = false): HTMLElement {
  const node = document.createElement('li');

  const mark = document.createElement('span');
  mark.className = 'glyph';
  mark.textContent = glyph;

  const text = document.createElement('span');
  text.className = 'label';
  text.textContent = label;

  node.append(mark, text);

  if (config) {
    const spacer = document.createElement('span');
    spacer.className = 'spacer';
    const gear = document.createElement('span');
    gear.className = 'row-config';
    gear.textContent = '⚙';
    node.append(spacer, gear);
  }

  return node;
}

async function renderEnvironment(workspace: string): Promise<void> {
  const name = workspace.replace(/\/+$/, '').split('/').pop() || workspace;
  const rows = [envNode('▣', name, true)];

  try {
    const { branch } = await bridge.gitBranch(workspace);
    if (branch) rows.push(envNode('⑂', branch));
  } catch {
    // Not a git repo, or git unavailable — just skip the branch row.
  }

  try {
    const parent = workspace.replace(/\/+$/, '').split('/').slice(0, -1).join('/') || '/';
    const siblings = await bridge.projects(parent);
    for (const project of siblings) {
      if (project.path === workspace) continue;
      const siblingName = project.path.replace(/\/+$/, '').split('/').pop() || project.path;
      rows.push(envNode('▢', siblingName));
    }
  } catch {
    // No sibling projects to list — the current workspace alone is fine.
  }

  try {
    const { repos } = await bridge.githubRepos();
    for (const repo of repos) rows.push(envNode('⊞', repo));
  } catch {
    // `gh` not installed or not logged in on this machine — skip silently.
  }

  el<HTMLUListElement>('env').replaceChildren(...rows);
}

/**
 * Token formatting as the reference shows it: 521.4k, 1M, 9.4k.
 * Tabular numerals keep the column from jittering as values tick up.
 */
function formatTokens(value: number): string {
  if (value >= 1_000_000) return `${(value / 1_000_000).toFixed(1).replace(/\.0$/, '')}M`;
  if (value >= 1_000) {
    const thousands = value / 1_000;
    return `${thousands >= 100 ? Math.round(thousands) : thousands.toFixed(1)}k`;
  }
  return String(value);
}

function renderBreakdown(): void {
  el<HTMLUListElement>('breakdown').replaceChildren();
}

function renderPanel(): void {
  el('connectors').textContent = 'nenhum conectado';
  renderBreakdown();
  el<HTMLUListElement>('changes').replaceChildren();
  void renderChanges();
}

function changeNode(pr: { title: string; repo: string; number: number; url: string; additions: number; deletions: number; files: number; ready: boolean }): HTMLElement {
  const node = document.createElement('li');
  node.className = 'change';
  node.innerHTML = [
    `<div class="change__title">${escapeHtml(pr.title)}</div>`,
    `<div class="change__repo">${escapeHtml(pr.repo)} #${pr.number}</div>`,
    '<div class="change__stat">',
    `<span class="change__add">+${pr.additions}</span>`,
    `<span class="change__del">-${pr.deletions}</span>`,
    `<span>${pr.files} arquivo${pr.files === 1 ? '' : 's'}</span>`,
    `<span class="change__status${pr.ready ? '' : ' change__status--draft'}" title="${pr.ready ? 'Pronto para revisão' : 'Rascunho'}"></span>`,
    '</div>',
  ].join('');
  node.addEventListener('click', () => shellOpen(pr.url));
  return node;
}

/** Opens a PR link in the OS browser; swallows the error since this panel is
 * informational, not a blocking action. */
function shellOpen(url: string): void {
  window.open(url, '_blank');
}

async function renderChanges(): Promise<void> {
  try {
    const { prs } = await bridge.githubPrs();
    if (prs.length === 0) {
      const { path: ghPath } = await bridge.which('gh').catch(() => ({ path: null }));
      if (!ghPath) {
        const li = document.createElement('li');
        li.className = 'changes__empty';
        li.innerHTML = 'GitHub CLI não encontrado. <a href="https://cli.github.com" data-action="open-external">Instalar gh</a>';
        el<HTMLUListElement>('changes').replaceChildren(li);
        return;
      }
    }
    el<HTMLUListElement>('changes').replaceChildren(...prs.map(changeNode));
  } catch {
    // `gh` not installed or not logged in — leave it empty.
  }
}

function renderStaticRegions(workspace: string): void {
  renderAllConversations([]);
  el<HTMLUListElement>('pinned').replaceChildren();
  el('groups').replaceChildren();
  void renderEnvironment(workspace);
  renderPanel();
  renderThemes();
  let savedTheme = 'dark';
  try {
    savedTheme = localStorage.getItem(THEME_KEY) ?? 'dark';
  } catch {
    savedTheme = 'dark';
  }
  applyTheme(savedTheme);
}

function renderTerminal(): void {
  const label = currentWorkspace.replace(/\/+$/, '').split('/').pop() || currentWorkspace;
  el('terminal-label').textContent = `Terminal ${label}`;
  const output = el<HTMLDivElement>('terminal-body');
  if (terminalWorkspaceDisplayed !== currentWorkspace) {
    terminalWorkspaceDisplayed = currentWorkspace;
    output.replaceChildren();
    const note = document.createElement('div');
    note.className = 'term-line term-line--note';
    note.textContent = 'Comandos são executados no workspace selecionado após sua confirmação.';
    output.append(note);
  }
  el<HTMLInputElement>('terminal-input').focus();
}

function appendTerminalLine(kind: 'prompt' | 'output' | 'note', text: string): void {
  const output = el<HTMLDivElement>('terminal-body');
  const line = document.createElement('div');
  line.className = `term-line term-line--${kind}`;
  line.textContent = text;
  output.append(line);
  output.scrollTop = output.scrollHeight;
}

async function runTerminalCommand(command: string): Promise<void> {
  appendTerminalLine('prompt', `${currentWorkspace} › ${command}`);
  const input = el<HTMLInputElement>('terminal-input');
  const button = el<HTMLButtonElement>('terminal-run');
  input.disabled = true;
  button.disabled = true;
  try {
    const result = await bridge.terminalRun(currentWorkspace, command);
    if (result.cancelled) {
      appendTerminalLine('note', 'Execução cancelada.');
      return;
    }
    if (result.stdout.trim()) appendTerminalLine('output', result.stdout.trimEnd());
    if (result.timedOut) appendTerminalLine('note', 'Tempo limite de 60 s atingido.');
    else appendTerminalLine('note', `Processo encerrado com código ${result.exitCode}.`);
  } catch (error) {
    appendTerminalLine('note', error instanceof Error ? error.message : String(error));
  } finally {
    input.disabled = false;
    button.disabled = false;
    input.focus();
  }
}

function setTerminal(open: boolean): void {
  layout.dataset.terminal = open ? 'open' : 'closed';
}

/* ── Theme ───────────────────────────────────────────────────── */

/**
 * Themes are colour-token sets defined entirely in CSS. This list only names
 * them, and each swatch carries its own `data-theme` so it reads the colours
 * straight from that theme's block — no palette is duplicated here.
 */
/** Light/dark only — the exact two modes Claude Desktop itself ships. */
const THEMES = [
  { id: 'dark', label: 'Escuro' },
  { id: 'light', label: 'Claro' },
] as const;

const THEME_KEY = 'repl:theme';

function applyTheme(id: string): void {
  const known = THEMES.some((theme) => theme.id === id) ? id : 'dark';
  document.documentElement.dataset.theme = known;
  for (const child of el('themes').children) {
    child.classList.toggle('is-active', (child as HTMLElement).dataset.theme === known);
  }
  try {
    localStorage.setItem(THEME_KEY, known);
  } catch {
    // file:// can refuse storage; the theme still applies for this session.
  }
}

function renderThemes(): void {
  el('themes').replaceChildren(
    ...THEMES.map(({ id, label }) => {
      const swatch = document.createElement('button');
      swatch.className = 'theme';
      swatch.dataset.theme = id;
      swatch.title = `Tema ${label}`;
      swatch.setAttribute('role', 'radio');
      swatch.setAttribute('aria-label', `Tema ${label}`);
      return swatch;
    }),
  );
}

/* ── Account and provider settings ─────────────────────────────── */

function initials(name: string): string {
  const trimmed = name.trim();
  if (!trimmed) return '?';
  return trimmed.charAt(0).toUpperCase();
}

let currentAccountEmail = '';

function renderAccount(account: { name: string; email?: string; plan?: string }): void {
  const mark = initials(account.name);
  currentAccountEmail = account.email ?? '';
  const displayName = account.plan ? `${account.name} · ${account.plan}` : account.name;
  el('sidebar-avatar').textContent = mark;
  el('sidebar-account-name').textContent = displayName;
  el('settings-avatar').textContent = mark;
  el('settings-account-name').textContent = displayName;
  el('settings-account-email').textContent = currentAccountEmail;
}

/**
 * Prefers the real, shared Claude Code login (same one `claude auth status`
 * reports) over the workspace's git identity, since it is the account that
 * actually authenticates chat turns. Falls back to git when logged out or
 * when `claude` isn't the active provider's CLI.
 */
async function resolveAccountIdentity(workspace: string): Promise<{ name: string; email?: string; plan?: string }> {
  const auth = await bridge.authStatus();
  if (auth.loggedIn && (auth.email || auth.orgName)) {
    const name = auth.orgName ?? auth.email ?? 'Conta Claude';
    const plan = auth.subscriptionType ? auth.subscriptionType.charAt(0).toUpperCase() + auth.subscriptionType.slice(1) : undefined;
    return { ...(auth.email ? { name, email: auth.email } : { name }), ...(plan ? { plan } : {}) };
  }
  return bridge.account(workspace);
}

async function openAccountMenu(): Promise<void> {
  const menu = el('account-menu');
  const opening = menu.hidden;
  menu.hidden = !menu.hidden;
  if (opening) el('account-menu-email').textContent = currentAccountEmail || '—';
}

async function signOut(): Promise<void> {
  el('account-menu').hidden = true;
  const confirmed = window.confirm(
    'Sair da conta do Claude Code? Isso desconecta o CLI usado por este app (e por qualquer outro projeto que use o mesmo login) até você entrar novamente.',
  );
  if (!confirmed) return;
  try {
    await bridge.authLogout();
    void loadSettings(currentWorkspace);
  } catch (error) {
    showNotice('Não foi possível sair', error instanceof Error ? error.message : String(error), true);
  }
}

/* ── Setup / onboarding panel ───────────────────────────────── */

function showSetupPanel(
  workspace: string,
  status: { has_claude_cli: boolean },
): void {
  const panel = el('setup-panel');
  panel.hidden = false;

  const radios = panel.querySelectorAll<HTMLInputElement>('input[name="setup-provider-radio"]');
  const keyLabel = el('setup-key-label');
  const claudeWarning = el('setup-claude-warning');
  const keyInput = el<HTMLInputElement>('setup-api-key');
  const saveBtn = el('setup-save-btn');
  const errorEl = el('setup-error');

  function updateProviderUI(): void {
    const provider = [...radios].find((r) => r.checked)?.value ?? 'minimax';
    keyLabel.textContent = provider === 'minimax' ? 'Chave MiniMax' : 'Chave Anthropic';
    keyInput.placeholder = provider === 'minimax' ? 'mxXXXXXXXXXXXXXX' : 'sk-ant-XXXXXXXX';
    claudeWarning.hidden = provider !== 'claude' || status.has_claude_cli;
  }

  for (const radio of radios) radio.addEventListener('change', updateProviderUI);
  updateProviderUI();

  saveBtn.addEventListener('click', async () => {
    const provider = ([...radios].find((r) => r.checked)?.value ?? 'minimax') as 'minimax' | 'claude';
    const apiKey = keyInput.value.trim();
    if (!apiKey) {
      errorEl.textContent = 'Insira a chave de API.';
      errorEl.hidden = false;
      return;
    }
    errorEl.hidden = true;
    saveBtn.setAttribute('disabled', 'true');
    try {
      await bridge.setCredentials(workspace, provider, apiKey);
      panel.hidden = true;
      void loadDcxData(workspace);
    } catch (error) {
      errorEl.textContent = error instanceof Error ? error.message : String(error);
      errorEl.hidden = false;
    } finally {
      saveBtn.removeAttribute('disabled');
    }
  });
}

function renderConfig(config: { provider: string; model?: string; web_enabled: boolean; max_tool_calls: number }): void {
  for (const input of el<HTMLDivElement>('settings-provider').querySelectorAll<HTMLInputElement>(
    'input[name="provider"]',
  )) {
    input.checked = input.value === config.provider;
  }
  el<HTMLInputElement>('settings-web').checked = config.web_enabled;
  el<HTMLInputElement>('settings-max-tool-calls').value = String(config.max_tool_calls);
  const providerLabel = config.provider === 'minimax' ? 'MiniMax' : 'Claude';
  const modelLabel = config.model ?? 'modelo padrão';
  el('settings-model').textContent = `Modelo: ${providerLabel} · ${modelLabel}`;
  el('model-state').textContent = `${providerLabel} · ${modelLabel}`;
}

async function loadSettings(workspace: string): Promise<void> {
  try {
    const [account, config] = await Promise.all([resolveAccountIdentity(workspace), bridge.config(workspace)]);
    renderAccount(account);
    renderConfig(config);
  } catch (error) {
    showNotice('Configuração indisponível', error instanceof Error ? error.message : String(error), true);
  }
}

async function updateConfig(updates: Partial<{ provider: string; web_enabled: boolean; max_tool_calls: number }>): Promise<void> {
  try {
    const config = await bridge.setConfig(currentWorkspace, updates);
    renderConfig(config);
  } catch (error) {
    showNotice('Não foi possível salvar', error instanceof Error ? error.message : String(error), true);
  }
}

/* ── Background tasks ────────────────────────────────────────── */

/* Cloned so the mock stays a seed rather than mutable render state. */
const backgroundTasks: BackgroundTask[] = [];
let backgroundCompleted = 0;

function formatElapsed(seconds: number): string {
  if (seconds < 60) return `${seconds} s`;
  return `${Math.floor(seconds / 60)}m ${seconds % 60}s`;
}

function taskRow(task: BackgroundTask): HTMLElement {
  const node = row(
    'bgtask',
    [
      `<span class="bgtask__glyph">${escapeHtml(task.glyph)}</span>`,
      `<span class="bgtask__title">${escapeHtml(task.title)}</span>`,
      `<span class="bgtask__elapsed">${formatElapsed(task.elapsed)}</span>`,
      '<button class="toggle toggle--sm" role="switch" aria-checked="true" data-bg-stop title="Interromper"><span></span></button>',
    ].join(''),
  );
  node.dataset.bgId = task.id;
  return node;
}

function renderBackground(): void {
  const running = backgroundTasks.filter((task) => task.running).map(taskRow);
  const done = row(
    'bgtask bgtask--done',
    `<span class="bgtask__glyph">◇</span><span class="bgtask__title">${backgroundCompleted} concluídas</span>`,
  );
  el('bg-tasks').replaceChildren(...running, done);
}

function stopBackgroundTask(id: string): void {
  const task = backgroundTasks.find((entry) => entry.id === id);
  if (!task?.running) return;
  task.running = false;
  backgroundCompleted += 1;
  renderBackground();
  if (currentWorkspace) void bridge.cancelRun(currentWorkspace, id).catch(() => {});
}

function addRunTask(runId: string, title: string): void {
  backgroundTasks.push({ id: runId, glyph: '>_', title, elapsed: 0, running: true });
  renderBackground();
}

function finishRunTask(runId: string): void {
  const task = backgroundTasks.find((entry) => entry.id === runId);
  if (!task?.running) return;
  task.running = false;
  backgroundCompleted += 1;
  renderBackground();
}

window.setInterval(() => {
  let changed = false;
  for (const task of backgroundTasks) {
    if (task.running) {
      task.elapsed += 1;
      changed = true;
    }
  }
  if (changed) renderBackground();
}, 1000);

/* ── Task queues ─────────────────────────────────────────────── */

function loopNode(summary: DcxLoopSummary): HTMLElement {
  const view = describeLoop(summary);
  const node = document.createElement('li');
  node.className = `loop loop--${summary.status}`;
  node.dataset.loopId = summary.id;
  const action = view.canPause
    ? '<button class="btn btn--small" data-loop-action="pause">Pausar</button>'
    : view.canResume
      ? '<button class="btn btn--small" data-loop-action="resume">Retomar</button>'
      : '';
  node.innerHTML = [
    '<div class="loop__head">',
    `<span class="loop__status">${escapeHtml(view.statusLabel)}</span>`,
    `<span class="loop__progress">${escapeHtml(view.progress)}</span>`,
    action,
    '</div>',
    '<div class="gauge__bar"><div class="gauge__fill"></div></div>',
    view.detail ? `<div class="loop__detail">${escapeHtml(view.detail)}</div>` : '',
    `<div class="loop__spend">${escapeHtml(view.spend)}</div>`,
  ].join('');
  node.querySelector<HTMLElement>('.gauge__fill')!.style.width = `${view.percent}%`;
  return node;
}

async function renderLoops(): Promise<void> {
  if (!currentWorkspace) return;
  try {
    const { loops } = await bridge.loops(currentWorkspace);
    const list = el<HTMLUListElement>('loops');
    if (loops.length === 0) {
      const empty = document.createElement('li');
      empty.className = 'loop loop--empty';
      empty.textContent = 'Nenhuma fila neste workspace.';
      list.replaceChildren(empty);
      return;
    }
    list.replaceChildren(...loops.map(loopNode));
  } catch {
    // Daemon offline: keep the last known state; the connection badge already says so.
  }
}

async function handleLoopAction(button: HTMLElement): Promise<void> {
  const id = button.closest<HTMLElement>('[data-loop-id]')?.dataset.loopId;
  if (!id || !currentWorkspace) return;
  button.setAttribute('disabled', '');
  try {
    if (button.dataset.loopAction === 'pause') await bridge.pauseLoop(currentWorkspace, id);
    else await bridge.resumeLoop(currentWorkspace, id);
  } catch (error) {
    showNotice('Não foi possível controlar a fila', error instanceof Error ? error.message : String(error), true);
  }
  await renderLoops();
}

window.setInterval(() => void renderLoops(), 5000);

/* ── Transcript ──────────────────────────────────────────────── */

function row(cls: string, html: string): HTMLElement {
  const node = document.createElement('div');
  node.className = cls;
  node.innerHTML = html;
  return node;
}

function escapeHtml(value: string): string {
  return value
    .replace(/&/g, '&amp;')
    .replace(/</g, '&lt;')
    .replace(/>/g, '&gt;')
    .replace(/"/g, '&quot;');
}

function firstLine(text: string): string {
  const line = text
    .replace(/\\n/g, '\n')
    .split('\n')
    .find((value) => value.trim() !== '')
    ?.trim() ?? '';
  return line.length > 140 ? `${line.slice(0, 137)}…` : line;
}

/**
 * Inline formatting for already-escaped text.
 *
 * Escaping happens first and formatting runs on the escaped string, so the
 * only markup that can reach the DOM is the handful of tags emitted here.
 */
function inlineMarkup(escaped: string): string {
  return (
    escaped
      .replace(/\*\*(.+?)\*\*/g, '<strong>$1</strong>')
      .replace(/`(.+?)`/g, '<code>$1</code>')
      // Rendered as a span, not an anchor: the mock points at real files but
      // there is nothing to navigate to, and a dead href would be worse than
      // no affordance at all.
      .replace(/\[(.+?)\]\((.+?)\)/g, '<span class="link">$1</span>')
  );
}

const BULLET = /^\s*[-*]\s+(.*)$/;
const NUMBERED = /^\s*\d+\.\s+(.*)$/;

type ListTag = 'ul' | 'ol';

interface ListItem {
  tag: ListTag;
  content: string;
}

function matchListItem(line: string): ListItem | null {
  const bullet = BULLET.exec(line);
  if (bullet) return { tag: 'ul', content: bullet[1] ?? '' };

  const numbered = NUMBERED.exec(line);
  if (numbered) return { tag: 'ol', content: numbered[1] ?? '' };

  return null;
}

/**
 * A deliberately small markdown subset: paragraphs, `-` bullets, `1.`
 * ordered lists, plus the inline pass above.
 */
function blockMarkup(text: string): string {
  const out: string[] = [];
  let open: ListTag | null = null;

  for (const line of escapeHtml(text).split('\n')) {
    const item = matchListItem(line);

    if (!item) {
      if (open) {
        out.push(`</${open}>`);
        open = null;
      }
      // Blank lines only ever close a list; they emit nothing themselves.
      if (line.trim() !== '') out.push(`<p>${inlineMarkup(line)}</p>`);
      continue;
    }

    if (open !== item.tag) {
      if (open) out.push(`</${open}>`);
      out.push(`<${item.tag}>`);
      open = item.tag;
    }
    out.push(`<li>${inlineMarkup(item.content)}</li>`);
  }

  if (open) out.push(`</${open}>`);
  return out.join('');
}

function addUserMessage(text: string): void {
  flushAssistantStream();
  clearThinking();
  streamingAssistantBody = undefined;
  streamingAssistantText = '';
  streamingPendingText = '';
  thread.append(row('msg msg--user', `<div class="msg__body">${escapeHtml(text)}</div>`));
  scrollToEnd();
}

function addAssistantMessage(text: string, merge = true): void {
  if (merge) {
    streamingPendingText += text;
    const hasBoundary = /(?:\n\s*\n|[.!?。！？](?:\s|$))/.test(streamingPendingText);
    if (streamingPendingText.length >= 180 || hasBoundary) flushAssistantStream();
    else scheduleAssistantFlush();
    return;
  }
  const node = row('msg', `<div class="msg__body">${blockMarkup(text)}</div>`);
  thread.append(node);
  if (merge) {
    streamingAssistantBody = node.querySelector<HTMLElement>('.msg__body') ?? undefined;
    streamingAssistantText = text;
  }
  scrollToEnd();
}

function scheduleAssistantFlush(): void {
  if (streamFlushTimer !== undefined) return;
  streamFlushTimer = window.setTimeout(() => {
    streamFlushTimer = undefined;
    flushAssistantStream();
  }, 120);
}

function flushAssistantStream(): void {
  if (streamFlushTimer !== undefined) {
    window.clearTimeout(streamFlushTimer);
    streamFlushTimer = undefined;
  }
  if (!streamingPendingText) return;
  streamingAssistantText += streamingPendingText;
  streamingPendingText = '';
  if (!streamingAssistantBody) {
    const node = row('msg', '<div class="msg__body"></div>');
    thread.append(node);
    streamingAssistantBody = node.querySelector<HTMLElement>('.msg__body') ?? undefined;
  }
  if (streamingAssistantBody) streamingAssistantBody.innerHTML = blockMarkup(streamingAssistantText);
  scrollToEnd();
}

function addActivity(id: string, name: string, input: unknown): HTMLElement {
  setThinking(true);
  const label = describeActivity(name, input);
  const node = row('action-step action-step--running',
    `<span class="action-step__mark" aria-hidden="true">◌</span><span class="action-step__label">${escapeHtml(label)}</span>`,
  );
  node.dataset.id = id;
  (thinkingBody ?? thread).append(node);
  setActivityStatus(label);
  scrollToEnd();
  return node;
}

function finishActivity(id: string, ok: boolean, detail: string): void {
  const node = thread.querySelector<HTMLElement>(`[data-id="${CSS.escape(id)}"]`);
  if (!node) return;

  node.classList.remove('action-step--running');
  node.classList.add(ok ? 'action-step--done' : 'action-step--error');

  const mark = node.querySelector('.action-step__mark');
  if (mark) mark.textContent = ok ? '✓' : '×';
  if (detail) node.title = firstLine(detail);
  setActivityStatus('Preparando resposta…');
}

function setActivityStatus(label: string, resetClock = false): void {
  if (!thinkingNode) return;
  const status = thinkingNode.querySelector<HTMLElement>('.thinking__label');
  const elapsed = thinkingNode.querySelector<HTMLElement>('.thinking__elapsed');
  if (status) status.textContent = label;
  if (resetClock || activityStartedAt === 0) activityStartedAt = Date.now();
  if (elapsed) elapsed.textContent = formatActivityElapsed(Date.now() - activityStartedAt);
  if (activityTimer === undefined) {
    activityTimer = window.setInterval(() => {
      const current = thinkingNode?.querySelector<HTMLElement>('.thinking__elapsed');
      if (current && activityStartedAt) current.textContent = formatActivityElapsed(Date.now() - activityStartedAt);
    }, 1000);
  }
}

function clearThinking(): void {
  if (activityTimer !== undefined) {
    window.clearInterval(activityTimer);
    activityTimer = undefined;
  }
  thinkingNode?.remove();
  thinkingNode = undefined;
  thinkingBody = undefined;
  activityStartedAt = 0;
}

function scrollToEnd(): void {
  const scroller = el<HTMLDivElement>('thread');
  scroller.scrollTop = scroller.scrollHeight;
}

function showNotice(title: string, body: string, danger = false): void {
  const host = el<HTMLDivElement>('notice');
  host.hidden = false;
  host.innerHTML = `
    <div class="notice ${danger ? 'notice--danger' : ''}">
      <div class="notice__title">${escapeHtml(title)}</div>
      <div class="msg__body">${escapeHtml(body)}</div>
      <div class="notice__actions">
        <button class="btn btn--primary" data-action="retry">Tentar novamente</button>
      </div>
    </div>`;
}

/* ── Context gauge ───────────────────────────────────────────── */

async function updateContext(used: number): Promise<void> {
  const info = await bridge.context(used);
  ctxFill.style.width = `${info.percent}%`;
  ctxPct.textContent = `${info.percent}% usado`;
  ctxCount.textContent = `${formatTokens(info.used)} / ${formatTokens(info.limit)}`;
  const headroom = el('ctx-headroom');
  headroom.textContent = `${formatTokens(info.limit - info.used)} até o auto-compact`;
  return renderContextBreakdown(used, info.limit);
}

interface UsageTotals {
  messages: number;
  cacheCreate: number;
  output: number;
}

/**
 * Sums the real per-turn usage the Claude CLI reports (cache_read_input_tokens
 * doubles as "history re-sent this turn", which is why it is folded into
 * "Mensagens" rather than its own row — it has no separate meaning to a user).
 */
function sumUsage(messages: Array<{ input_tokens?: number; output_tokens?: number; cache_creation_input_tokens?: number; cache_read_input_tokens?: number }>): UsageTotals {
  let messagesTotal = 0;
  let cacheCreate = 0;
  let output = 0;
  for (const message of messages) {
    messagesTotal += (message.input_tokens ?? 0) + (message.cache_read_input_tokens ?? 0);
    cacheCreate += message.cache_creation_input_tokens ?? 0;
    output += message.output_tokens ?? 0;
  }
  return { messages: messagesTotal, cacheCreate, output };
}

function breakdownRow(label: string, tokens: number, pct: number, color?: string): HTMLElement {
  const node = document.createElement('li');
  if (!color) node.classList.add('is-headroom');
  node.innerHTML = [
    '<span class="swatch"></span>',
    `<span class="label">${escapeHtml(label)}</span>`,
    `<span class="tokens">${formatTokens(tokens)}</span>`,
    `<span class="pct">${color ? `${pct}%` : ''}</span>`,
  ].join('');
  if (color) node.querySelector<HTMLElement>('.swatch')!.style.background = color;
  return node;
}

function renderContextBreakdown(used: number, limit: number): void {
  const totals = lastUsageTotals;
  const rows: Array<{ label: string; tokens: number; color: string }> = [
    { label: 'Mensagens', tokens: totals.messages, color: '#5b8def' },
    { label: 'Cache criado', tokens: totals.cacheCreate, color: '#c8963e' },
    { label: 'Respostas', tokens: totals.output, color: '#7f9b7a' },
  ].filter((row) => row.tokens > 0);
  el<HTMLUListElement>('breakdown').replaceChildren(
    ...rows.map((row) => breakdownRow(row.label, row.tokens, used ? Math.round((row.tokens / used) * 100) : 0, row.color)),
    breakdownRow('Espaço livre', Math.max(0, limit - used), 0),
  );
}

let lastUsageTotals: UsageTotals = { messages: 0, cacheCreate: 0, output: 0 };

async function refreshContextFromSession(workspace: string, sessionId: string): Promise<void> {
  try {
    const session = await bridge.session(workspace, sessionId);
    lastUsageTotals = sumUsage(session.messages);
    accumulatedTokens = lastUsageTotals.messages + lastUsageTotals.cacheCreate + lastUsageTotals.output;
    await updateContext(accumulatedTokens);
  } catch {
    // Best-effort: the gauge just keeps its last known value.
  }
}

/* ── Events ──────────────────────────────────────────────────── */

let accumulatedTokens = 0;

bridge.onEvent((payload) => {
  switch (payload.type) {
    case 'session': {
      const isNewSession = currentSessionId !== payload.sessionId;
      currentSessionId = payload.sessionId;
      if (isNewSession && currentWorkspace) {
        el<HTMLButtonElement>('promote-run').hidden = false;
        void bridge.sessions(currentWorkspace).then(renderSessionLists);
      }
      break;
    }

    case 'tool_start':
      flushAssistantStream();
      if (payload.id) addActivity(payload.id, payload.name ?? 'tool', payload.input);
      break;

    case 'progress': {
      setThinking(true);
      setActivityStatus(describeProgress(payload.message ?? ''));
      break;
    }

    case 'tool_end':
      flushAssistantStream();
      if (payload.id)
        finishActivity(payload.id, payload.ok ?? false, firstLine(payload.output ?? ''));
      break;

    case 'denied':
      if (payload.id) finishActivity(payload.id, false, payload.reason ?? '');
      break;

    case 'text':
      addAssistantMessage(payload.text ?? '');
      break;

    case 'done': {
      flushAssistantStream();
      if (currentWorkspace && currentSessionId) void refreshContextFromSession(currentWorkspace, currentSessionId);
      busy = false;
      setPlaceholder();
      setThinking(false, 'Concluído');
      break;
    }

    case 'error':
      flushAssistantStream();
      showNotice('Turno interrompido', payload.message ?? 'erro desconhecido', true);
      busy = false;
      setPlaceholder();
      setThinking(false, 'Interrompido');
      break;

    case 'run_event':
      if (payload.event === 'output' && typeof payload.data?.text === 'string') {
        addAssistantMessage(payload.data.text);
      } else if (payload.event === 'awaiting_input') {
        flushAssistantStream();
        showNotice('Run aguardando resposta', String(payload.data?.text ?? 'Responda pela interface.'), false);
      } else if (payload.event === 'done') {
        flushAssistantStream();
        addAssistantMessage('Run concluído.');
        if (payload.runId) finishRunTask(payload.runId);
        busy = false;
        setPlaceholder();
        setThinking(false, 'Run concluído');
      } else if (payload.event === 'failed' || payload.event === 'cancelled') {
        showNotice('Run encerrado', payload.event, payload.event === 'failed');
        if (payload.runId) finishRunTask(payload.runId);
        busy = false;
        setPlaceholder();
        setThinking(false, 'Run encerrado');
      } else if (payload.event === 'recovered') {
        showNotice('Run retomado', 'O daemon reiniciou e o trabalho continuou de onde parou.', false);
      } else if (payload.event === 'recovery_stopped' || payload.event === 'recovery_failed') {
        showNotice('Run interrompido', String(payload.data?.reason ?? 'Não foi possível retomar.'), true);
        if (payload.runId) finishRunTask(payload.runId);
        busy = false;
        setPlaceholder();
        setThinking(false, 'Run interrompido');
      }
      break;

    default:
      break;
  }
});

function setPlaceholder(): void {
  input.placeholder = 'Digite / para comandos';
}

function setThinking(active: boolean, doneLabel = 'Concluído'): void {
  if (active && !thinkingNode) {
    thinkingNode = row(
      'thinking',
      `<div class="thinking__body action-steps" aria-live="polite"></div><button class="thinking__summary" type="button" aria-expanded="true">${activityIndicatorMarkup()}<span class="thinking__label">Analisando seu pedido…</span><span class="thinking__elapsed">0 s</span><span class="thinking__chevron" aria-hidden="true">›</span></button>`,
    );
    thinkingBody = thinkingNode.querySelector<HTMLElement>('.thinking__body') ?? undefined;
    thread.append(thinkingNode);
    activityStartedAt = Date.now();
    setActivityStatus('Analisando seu pedido…');
    scrollToEnd();
  } else if (active && thinkingNode) {
    if (thinkingNode.classList.contains('thinking--done')) {
      thinkingNode.classList.remove('thinking--done', 'thinking--error');
      setActivityStatus('Analisando seu pedido…', true);
      const spinner = thinkingNode.querySelector<HTMLElement>('.thinking__spinner');
      if (spinner) spinner.outerHTML = activityIndicatorMarkup();
    }
  } else if (!active && thinkingNode) {
    const failed = doneLabel !== 'Concluído' && doneLabel !== 'Run concluído';
    thinkingNode.classList.toggle('thinking--done', !failed);
    thinkingNode.classList.toggle('thinking--error', failed);
    setActivityStatus(doneLabel);
    const spinner = thinkingNode.querySelector<HTMLElement>('.thinking__spinner');
    if (spinner) spinner.textContent = failed ? '×' : '✓';
    if (activityTimer !== undefined) {
      window.clearInterval(activityTimer);
      activityTimer = undefined;
    }
  }
}

async function promoteCurrentSession(): Promise<void> {
  if (!currentWorkspace || !currentSessionId || busy) return;
  try {
    const preview = await bridge.promote(currentWorkspace, currentSessionId);
    if (!window.confirm(`Promover esta conversa para execução?\n\n${preview.request}`)) return;
    await bridge.approve(currentWorkspace, currentSessionId);
    const started = await bridge.start(currentWorkspace, currentSessionId);
    busy = true;
    setThinking(true);
    setPlaceholder();
    addRunTask(started.run_id, firstLine(preview.request));
    showNotice('Run iniciado', `Job ${started.run_id} em execução.`, false);
  } catch (error) {
    showNotice('Não foi possível iniciar o run', error instanceof Error ? error.message : String(error), true);
  }
}

/* ── Input ───────────────────────────────────────────────────── */

function autoGrow(): void {
  input.style.height = 'auto';
  input.style.height = `${Math.min(input.scrollHeight, 200)}px`;
}

function renderAttachChips(): void {
  const box = input.closest('.composer__box')!;
  box.querySelectorAll('.attach-chip').forEach((c) => c.remove());
  for (const filePath of attachedFiles) {
    const chip = document.createElement('span');
    chip.className = 'attach-chip';
    chip.title = filePath;
    chip.textContent = filePath.split('/').pop() ?? filePath;
    const rm = document.createElement('button');
    rm.textContent = '✕';
    rm.addEventListener('click', () => {
      attachedFiles = attachedFiles.filter((f) => f !== filePath);
      renderAttachChips();
    });
    chip.appendChild(rm);
    box.prepend(chip);
  }
}

async function submit(): Promise<void> {
  const rawPrompt = input.value.trim();
  if (rawPrompt === '' || busy) return;
  const prompt = attachedFiles.length > 0
    ? `[Arquivos anexados: ${attachedFiles.join(', ')}]\n\n${rawPrompt}`
    : rawPrompt;
  attachedFiles = [];
  renderAttachChips();

  busy = true;
  input.value = '';
  autoGrow();
  setPlaceholder();
  el<HTMLDivElement>('notice').hidden = true;
  addUserMessage(rawPrompt);
  setThinking(true);

  try {
    await bridge.run(prompt, currentWorkspace, currentSessionId, reposForWorkspace());
  } catch (error) {
    showNotice(
      'Falha ao iniciar o turno',
      error instanceof Error ? error.message : String(error),
      true,
    );
    busy = false;
    setPlaceholder();
    setThinking(false, 'Interrompido');
  }
}

input.addEventListener('input', autoGrow);

el<HTMLFormElement>('routine-editor').addEventListener('submit', (event) => {
  void saveCurrentRoutine(event);
});

el<HTMLFormElement>('terminal-form').addEventListener('submit', (event) => {
  event.preventDefault();
  const terminalInput = el<HTMLInputElement>('terminal-input');
  const command = terminalInput.value.trim();
  if (!command) return;
  terminalInput.value = '';
  void runTerminalCommand(command);
});

input.addEventListener('keydown', (event) => {
  if (event.key === 'Enter' && !event.shiftKey) {
    event.preventDefault();
    void submit();
  }
});

/* ── Chrome ──────────────────────────────────────────────────── */

function handleChromeAction(action: string): void {
  switch (action) {
    case 'search':
      openSearch();
      return;
    case 'routines':
      openRoutines();
      return;
    case 'close-routines':
      el<HTMLElement>('routines-view').hidden = true;
      return;
    case 'new-routine':
      setRoutineEditor();
      el<HTMLInputElement>('routine-title').focus();
      return;
    case 'refresh-routines':
      void loadRoutines(currentRoutineId, false);
      return;
    case 'close-search':
      el<HTMLElement>('search-view').hidden = true;
      return;
    case 'artifacts':
      void openArtifacts();
      return;
    case 'close-artifacts':
      el<HTMLElement>('artifacts-view').hidden = true;
      return;
    case 'refresh-artifacts':
      void loadArtifactCatalog();
      return;
    case 'toggle-sidebar':
      layout.dataset.sidebar = layout.dataset.sidebar === 'hidden' ? 'shown' : 'hidden';
      return;
    case 'toggle-panel':
      layout.dataset.panel = layout.dataset.panel === 'hidden' ? 'shown' : 'hidden';
      return;
    case 'terminal':
      if (layout.dataset.terminal !== 'open') renderTerminal();
      setTerminal(layout.dataset.terminal !== 'open');
      return;
    case 'terminal-restart':
      void bridge.terminalRestart(currentWorkspace).then(() => {
        terminalWorkspaceDisplayed = '';
        renderTerminal();
      }).catch((error: unknown) => {
        showNotice('Não foi possível reiniciar o shell', error instanceof Error ? error.message : String(error), true);
      });
      return;
    case 'toggle-terminal':
      setTerminal(false);
      return;
    case 'toggle-bg':
      el('bg-section').classList.toggle('is-collapsed');
      return;
    case 'settings':
      el('theme-settings').hidden = false;
      void loadSettings(currentWorkspace);
      return;
    case 'close-settings':
      el('theme-settings').hidden = true;
      return;
    case 'open-setup':
      el('theme-settings').hidden = true;
      void bridge
        .setup(currentWorkspace)
        .catch(() => ({ provider: null, has_minimax_key: false, has_anthropic_key: false, has_claude_cli: false }))
        .then((setupStatus) => showSetupPanel(currentWorkspace, setupStatus));
      return;
    case 'account-menu':
      void openAccountMenu();
      return;
    case 'sign-out':
      void signOut();
      return;
    case 'promote':
      void promoteCurrentSession();
      return;
    case 'new':
      el<HTMLElement>('search-view').hidden = true;
      el<HTMLElement>('artifacts-view').hidden = true;
      el<HTMLElement>('routines-view').hidden = true;
      thread.replaceChildren();
      currentSessionId = undefined;
      accumulatedTokens = 0;
      lastUsageTotals = { messages: 0, cacheCreate: 0, output: 0 };
      void updateContext(0);
      el<HTMLButtonElement>('promote-run').hidden = true;
      document.querySelectorAll('[data-session-id].is-active').forEach((node) => node.classList.remove('is-active'));
      input.focus();
      return;
    case 'compact-session':
      if (!currentSessionId) return;
      void bridge.compactSession(currentWorkspace, currentSessionId).then((result) => {
        showNotice(`Sessão compactada`, `${result.removed} mensagens removidas, ${result.kept} mantidas.`);
        void loadSession(currentWorkspace, currentSessionId!);
      }).catch((error: unknown) => {
        showNotice('Erro ao compactar', error instanceof Error ? error.message : String(error), true);
      });
      return;
    case 'personalizar':
      el('theme-settings').hidden = false;
      void loadSettings(currentWorkspace);
      return;
    case 'back':
      if (sessionHistoryIndex > 0) {
        sessionHistoryIndex--;
        void loadSession(currentWorkspace, sessionHistory[sessionHistoryIndex]!);
      }
      return;
    case 'forward':
      if (sessionHistoryIndex < sessionHistory.length - 1) {
        sessionHistoryIndex++;
        void loadSession(currentWorkspace, sessionHistory[sessionHistoryIndex]!);
      }
      return;
    case 'reload':
      if (currentSessionId) void loadSession(currentWorkspace, currentSessionId);
      return;
    case 'toggle-code':
      layout.dataset.code = layout.dataset.code === 'shown' ? 'hidden' : 'shown';
      return;
    case 'open-shortcuts':
      showNotice('Atalhos', 'Enter → enviar · Shift+Enter → nova linha · Esc → cancelar');
      return;
    case 'toggle-web':
      void bridge.config(currentWorkspace).then((config) => {
        void bridge.setConfig(currentWorkspace, { web_enabled: !config.web_enabled }).then(() => {
          showNotice(config.web_enabled ? 'Busca web desativada' : 'Busca web ativada', '');
        });
      }).catch((error: unknown) => {
        showNotice('Erro', error instanceof Error ? error.message : String(error), true);
      });
      return;
    case 'mais':
    case 'mais-menu': {
      const maisMenu = document.getElementById('mais-menu');
      if (maisMenu) maisMenu.hidden = !maisMenu.hidden;
      return;
    }
    case 'open-external':
      return;
    default:
  }
}

document.addEventListener('change', (event) => {
  const target = event.target as HTMLElement;

  if (target.matches('input[name="provider"]')) {
    void updateConfig({ provider: (target as HTMLInputElement).value });
    return;
  }
  if (target.id === 'settings-web') {
    void updateConfig({ web_enabled: (target as HTMLInputElement).checked });
    return;
  }
  if (target.id === 'settings-max-tool-calls') {
    const value = Number.parseInt((target as HTMLInputElement).value, 10);
    if (Number.isFinite(value) && value > 0) void updateConfig({ max_tool_calls: value });
    return;
  }
});

document.addEventListener('click', (event) => {
  const target = event.target as HTMLElement;

  const loopButton = target.closest<HTMLElement>('[data-loop-action]');
  if (loopButton) {
    void handleLoopAction(loopButton);
    return;
  }

  if (target.closest('.composer__attach')) {
    void bridge.pickFile().then((paths) => {
      attachedFiles = [...attachedFiles, ...paths];
      renderAttachChips();
    }).catch(() => {});
    return;
  }

  if (target.closest('.composer__mic')) {
    const win = window as unknown as Record<string, unknown>;
    const SR = (win['SpeechRecognition'] ?? win['webkitSpeechRecognition']) as (new () => { lang: string; start(): void; onresult: ((ev: { results: ArrayLike<ArrayLike<{ transcript: string }>> }) => void) | null; onerror: (() => void) | null }) | undefined;
    if (!SR) {
      showNotice('Voz não suportada', 'Este navegador não suporta reconhecimento de voz.');
      return;
    }
    const recognition = new SR();
    recognition.lang = 'pt-BR';
    recognition.onresult = (ev) => {
      const transcript = ev.results[0]?.[0]?.transcript ?? '';
      input.value += transcript;
      input.dispatchEvent(new Event('input'));
    };
    recognition.onerror = () => showNotice('Voz', 'Não foi possível capturar áudio.');
    recognition.start();
    return;
  }

  const action = target.closest<HTMLElement>('[data-action]')?.dataset.action;
  if (action === 'open-external') {
    const anchor = target.closest<HTMLAnchorElement>('a');
    const href = anchor?.href;
    if (href) void bridge.openExternal(href).catch(() => {});
    return;
  }
  if (action === 'add-context-repo') {
    event.stopPropagation();
    void addContextRepo();
    return;
  }
  if (action === 'add-workspace') {
    event.stopPropagation();
    void bridge.pickWorkspace().then((workspace) => {
      if (workspace) return switchWorkspace(workspace);
    }).catch((error: unknown) => {
      showNotice('Não foi possível abrir o workspace', error instanceof Error ? error.message : String(error), true);
    });
    return;
  }
  if (action === 'remove-context-repo') {
    event.stopPropagation();
    const repo = target.closest<HTMLElement>('[data-action="remove-context-repo"]')?.dataset.repoPath;
    if (repo) {
      saveRepos(currentWorkspace, reposForWorkspace().filter((entry) => entry !== repo));
      renderWorkspaceGroups(currentProjects);
      showNotice('Acesso removido', `${repo} não será incluído nas próximas mensagens.`, false);
    }
    return;
  }
  if (action === 'select-artifact-run') {
    const runId = target.closest<HTMLElement>('[data-action="select-artifact-run"]')?.dataset.runId;
    const runTitle = target.closest<HTMLElement>('.artifact-run')?.querySelector('.artifact-run__title')?.textContent ?? '';
    if (runId) void selectArtifactRun(runId, runTitle);
    return;
  }
  if (action === 'open-artifact-file') {
    const fileButton = target.closest<HTMLElement>('[data-action="open-artifact-file"]');
    if (fileButton?.dataset.runId && fileButton.dataset.artifactName) {
      void openArtifactFile(fileButton.dataset.runId, fileButton.dataset.artifactName);
    }
    return;
  }
  if (action === 'open-search-result') {
    const sessionId = target.closest<HTMLElement>('[data-action="open-search-result"]')?.dataset.sessionId;
    if (sessionId) {
      el<HTMLElement>('search-view').hidden = true;
      void loadSession(currentWorkspace, sessionId);
    }
    return;
  }
  if (action === 'select-routine') {
    const id = target.closest<HTMLElement>('[data-action="select-routine"]')?.dataset.routineId;
    if (id) {
      void bridge.routines(currentWorkspace).then(({ routines }) => {
        const selected = routines.find((routine) => routine.id === id);
        if (selected) setRoutineEditor(selected);
      });
    }
    return;
  }
  if (action === 'use-routine') {
    useCurrentRoutine();
    return;
  }
  if (action === 'delete-routine') {
    void deleteCurrentRoutine();
    return;
  }

  const workspaceNode = target.closest<HTMLElement>('[data-workspace-path]');
  if (workspaceNode?.dataset.workspacePath) {
    void switchWorkspace(workspaceNode.dataset.workspacePath);
    return;
  }

  const accountMenu = el('account-menu');
  if (!accountMenu.hidden && !accountMenu.contains(target) && !target.closest('[data-action="account-menu"]')) {
    accountMenu.hidden = true;
  }

  const thoughtSummary = target.closest<HTMLElement>('.thinking__summary');
  if (thoughtSummary) {
    const thought = thoughtSummary.closest<HTMLElement>('.thinking');
    if (thought) {
      const collapsed = thought.classList.toggle('is-collapsed');
      thoughtSummary.setAttribute('aria-expanded', String(!collapsed));
    }
    return;
  }

  const pinToggle = target.closest<HTMLElement>('[data-action="toggle-pin"]');
  if (pinToggle) {
    event.stopPropagation();
    const sessionId = pinToggle.closest<HTMLElement>('[data-session-id]')?.dataset.sessionId;
    if (sessionId) void togglePin(sessionId, pinToggle.dataset.pinned !== 'true');
    return;
  }

  const sessionNode = target.closest<HTMLElement>('[data-session-id]');
  if (sessionNode?.dataset.sessionId) {
    void bridge.workspace().then((workspace) => loadSession(workspace, sessionNode.dataset.sessionId!));
    return;
  }

  const swatch = target.closest<HTMLElement>('.theme');
  if (swatch?.dataset.theme) {
    applyTheme(swatch.dataset.theme);
    return;
  }

  const stop = target.closest<HTMLElement>('[data-bg-stop]');
  if (stop) {
    const owner = stop.closest<HTMLElement>('[data-bg-id]');
    if (owner?.dataset.bgId) stopBackgroundTask(owner.dataset.bgId);
    return;
  }

  if (action) handleChromeAction(action);
});

el<HTMLInputElement>('session-search-input').addEventListener('input', (event) => {
  const query = (event.currentTarget as HTMLInputElement).value;
  void searchSessions(query);
});

async function loadDcxData(workspaceOverride?: string): Promise<void> {
  try {
    let savedWorkspace: string | null = null;
    try {
      savedWorkspace = localStorage.getItem('dcx.workspace');
    } catch {
      // Use the configured default when local storage is unavailable.
    }
    const workspace = workspaceOverride ?? savedWorkspace ?? await bridge.workspace();
    currentWorkspace = workspace;

    // Show onboarding if no credentials are configured.
    try {
      const setupStatus = await bridge.setup(workspace);
      if (!setupStatus.has_minimax_key && !setupStatus.has_anthropic_key) {
        showSetupPanel(workspace, setupStatus);
        return;
      }
    } catch {
      // Daemon may not be running yet; continue to show normal error.
    }

    renderStaticRegions(workspace);
    const connection = el('connection-state');
    connection.textContent = 'dcode conectado';
    void loadSettings(workspace);
    const sessions = await bridge.sessions(workspace);
    if (sessions[0]) await loadSession(workspace, sessions[0].session_id);
    renderSessionLists(sessions);
    const parent = workspace.replace(/\/+$/, '').split('/').slice(0, -1).join('/') || '/';
    currentProjects = await bridge.projects(parent);
    renderWorkspaceGroups(currentProjects);
    el('project-chip').firstChild!.textContent = workspace.replace(/\/+$/, '').split('/').pop() || workspace;
    if (!el<HTMLElement>('artifacts-view').hidden) void loadArtifactCatalog();
    if (!el<HTMLElement>('search-view').hidden) {
      void searchSessions(el<HTMLInputElement>('session-search-input').value);
    }
    if (!el<HTMLElement>('routines-view').hidden) void loadRoutines();
    void renderLoops();
  } catch (error) {
    el('connection-state').textContent = 'dcode offline';
    const detail = error instanceof Error ? error.message : String(error);
    const hint = detail.includes('ECONNREFUSED') || detail.includes('connect')
      ? 'O daemon dcode não está respondendo. Se você está em modo de desenvolvimento, inicie o daemon com: cargo run -p dcx-cli --bin dcxd -- --bind 127.0.0.1:8787'
      : detail;
    showNotice('dcode indisponível', hint, true);
    renderStaticRegions('Workspace não selecionado');
  }
}

void loadDcxData();
void updateContext(0);
setPlaceholder();
input.focus();
