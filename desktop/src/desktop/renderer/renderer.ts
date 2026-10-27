/**
 * Renderer.
 *
 * Holds no agent logic. It renders events that arrive over the preload bridge
 * and sends prompts back. The only reason this file exists is the DOM.
 */

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
  }>;
  projects(root: string): Promise<Array<{ path: string }>>;
  promote(workspace: string, sessionId: string): Promise<{ status: string; request: string }>;
  approve(workspace: string, sessionId: string): Promise<unknown>;
  start(workspace: string, sessionId: string): Promise<{ run_id: string; status: string }>;
  cancelRun(workspace: string, runId: string): Promise<void>;
  runCatalog(workspace: string): Promise<{ runs: Array<{ run_id: string; title: string; summary: string; status: string; criteria_passed: number | null; criteria_total: number | null; states: string[]; files: string[] }> }>;
  artifact(workspace: string, runId: string, name: string): Promise<{ name: string; content: string }>;
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
  terminalRun(workspace: string, command: string): Promise<{ cancelled: boolean; stdout: string; exitCode: number; timedOut: boolean }>;
  terminalRestart(workspace: string): Promise<boolean>;
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
let thoughtCount = 0;
let commandCount = 0;
let editCount = 0;
let currentProjects: Array<{ path: string }> = [];
let terminalWorkspaceDisplayed = '';
let selectedArtifactRun: string | undefined;

const CONTEXT_REPOS_KEY = 'harness.contextRepos';

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
  el('groups').replaceChildren(
    renderContextRepos(),
    projectSection({
      name: 'Workspaces',
      items: projects.map((project) => ({
        title: project.path.replace(/\/+$/, '').split('/').pop() || project.path,
        workspacePath: project.path,
        active: project.path === currentWorkspace,
      })),
    }),
  );
}

async function openArtifacts(): Promise<void> {
  el<HTMLElement>('artifacts-view').hidden = false;
  await loadArtifactCatalog();
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
      empty.textContent = 'Este workspace ainda não tem runs do Harness.';
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

async function loadSession(workspace: string, sessionId: string): Promise<void> {
  try {
    const session = await bridge.session(workspace, sessionId);
    currentWorkspace = workspace;
    currentSessionId = sessionId;
    el<HTMLButtonElement>('promote-run').hidden = false;
    thread.replaceChildren();
    streamingAssistantBody = undefined;
    streamingAssistantText = '';
    streamingPendingText = '';
    thinkingNode = undefined;
    thinkingBody = undefined;
    for (const message of session.messages) {
      if (message.role === 'user') addUserMessage(message.content);
      else addAssistantMessage(message.content, false);
    }
    document.querySelectorAll('[data-session-id]').forEach((node) => {
      node.classList.toggle('is-active', (node as HTMLElement).dataset.sessionId === sessionId);
    });
    lastUsageTotals = sumUsage(session.messages);
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
  add.setAttribute('aria-label', 'Adicionar');

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
  thread.replaceChildren();
  streamingAssistantBody = undefined;
  streamingAssistantText = '';
  streamingPendingText = '';
  thinkingNode = undefined;
  thinkingBody = undefined;
  accumulatedTokens = 0;
  lastUsageTotals = { messages: 0, cacheCreate: 0, output: 0 };
  el<HTMLButtonElement>('promote-run').hidden = true;
  void updateContext(0);
  try {
    localStorage.setItem('harness.workspace', workspace);
  } catch {
    // Keep the selection for this window when storage is unavailable.
  }
  await loadHarnessData(workspace);
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
    `<span class="change__status" style="background:${pr.ready ? 'var(--ok)' : 'var(--muted)'}" title="${pr.ready ? 'Pronto para revisão' : 'Rascunho'}"></span>`,
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
    el<HTMLUListElement>('changes').replaceChildren(...prs.map(changeNode));
  } catch {
    // `gh` not installed or not logged in on this machine — leave it empty.
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
  if (thinkingNode) {
    thinkingNode.remove();
    thinkingNode = undefined;
  }
  thinkingBody = undefined;
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

function addActivity(id: string, name: string, detail = ''): HTMLElement {
  setThinking(true);
  const lowerName = name.toLowerCase();
  thoughtCount += 1;
  if (lowerName.includes('run') || lowerName.includes('shell') || lowerName.includes('command')) commandCount += 1;
  if (lowerName.includes('edit') || lowerName.includes('write')) editCount += 1;
  updateThoughtSummary();
  const node = row(
    'activity activity--running',
    `<span class="activity__glyph">◆</span><span>${escapeHtml(name)}</span>${detail ? `<span class="activity__detail">${escapeHtml(detail)}</span>` : ''}`,
  );
  node.dataset.id = id;
  (thinkingBody ?? thread).append(node);
  scrollToEnd();
  return node;
}

function finishActivity(id: string, ok: boolean, detail: string): void {
  const node = thread.querySelector<HTMLElement>(`[data-id="${CSS.escape(id)}"]`);
  if (!node) return;

  node.classList.remove('activity--running');
  node.classList.add(ok ? 'activity--ok' : 'activity--error');

  const glyph = node.querySelector('.activity__glyph');
  if (glyph) glyph.textContent = ok ? '●' : '▲';

  let detailNode = node.querySelector('.activity__detail');
  if (!detailNode) {
    detailNode = document.createElement('span');
    detailNode.className = 'activity__detail';
    node.append(detailNode);
  }
  detailNode.textContent = detail;
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
    `<span class="swatch"${color ? ` style="background:${color}"` : ''}></span>`,
    `<span class="label">${escapeHtml(label)}</span>`,
    `<span class="tokens">${formatTokens(tokens)}</span>`,
    `<span class="pct">${color ? `${pct}%` : ''}</span>`,
  ].join('');
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
      if (payload.id) addActivity(payload.id, payload.name ?? 'tool', summarize(payload.input));
      break;

    case 'progress': {
      setThinking(true);
      const id = `progress-${Date.now()}-${thoughtCount}`;
      const node = addActivity(id, payload.message ?? 'Processando…');
      node.classList.remove('activity--running');
      node.classList.add('activity--ok');
      const glyph = node.querySelector('.activity__glyph');
      if (glyph) glyph.textContent = '◌';
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
      setThinking(false);
      addAssistantMessage(payload.text ?? '');
      break;

    case 'done': {
      flushAssistantStream();
      if (currentWorkspace && currentSessionId) void refreshContextFromSession(currentWorkspace, currentSessionId);
      busy = false;
      setPlaceholder();
      setThinking(false);
      break;
    }

    case 'error':
      flushAssistantStream();
      showNotice('Turno interrompido', payload.message ?? 'erro desconhecido', true);
      busy = false;
      setPlaceholder();
      setThinking(false);
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
        setThinking(false);
      } else if (payload.event === 'failed' || payload.event === 'cancelled') {
        showNotice('Run encerrado', payload.event, payload.event === 'failed');
        if (payload.runId) finishRunTask(payload.runId);
        busy = false;
        setPlaceholder();
        setThinking(false);
      }
      break;

    default:
      break;
  }
});

function summarize(value: unknown): string {
  const text = JSON.stringify(value) ?? '';
  return text.length > 70 ? `${text.slice(0, 67)}...` : text;
}

function setPlaceholder(): void {
  input.placeholder = 'Digite / para comandos';
}

function setThinking(active: boolean): void {
  if (active && !thinkingNode) {
    thoughtCount = 0;
    commandCount = 0;
    editCount = 0;
    thinkingNode = row(
      'thinking',
      '<button class="thinking__summary" type="button" aria-expanded="true"><span class="thinking__chevron">⌄</span><span class="thinking__label">Thought process</span><span class="thinking__meta"></span></button><div class="thinking__body"></div><div class="thinking__status"><span class="thinking__spinner">◌</span><span>Noodling…</span></div>',
    );
    thinkingBody = thinkingNode.querySelector<HTMLElement>('.thinking__body') ?? undefined;
    thread.append(thinkingNode);
    updateThoughtSummary();
    scrollToEnd();
  } else if (!active && thinkingNode) {
    thinkingNode.classList.add('thinking--done');
    const status = thinkingNode.querySelector<HTMLElement>('.thinking__status span:last-child');
    if (status) status.textContent = 'Concluído';
    updateThoughtSummary();
  }
}

function updateThoughtSummary(): void {
  const meta = thinkingNode?.querySelector<HTMLElement>('.thinking__meta');
  if (!meta) return;
  const parts = [`Thought ${Math.max(1, thoughtCount)} ${thoughtCount === 1 ? 'time' : 'times'}`];
  if (commandCount) parts.push(`Ran ${commandCount} ${commandCount === 1 ? 'command' : 'commands'}`);
  if (editCount) parts.push(`edited ${editCount} ${editCount === 1 ? 'file' : 'files'}`);
  meta.textContent = parts.join(' · ');
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

async function submit(): Promise<void> {
  const prompt = input.value.trim();
  if (prompt === '' || busy) return;

  busy = true;
  input.value = '';
  autoGrow();
  setPlaceholder();
  el<HTMLDivElement>('notice').hidden = true;
  addUserMessage(prompt);
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
    setThinking(false);
  }
}

input.addEventListener('input', autoGrow);

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
      thread.replaceChildren();
      currentSessionId = undefined;
      accumulatedTokens = 0;
      lastUsageTotals = { messages: 0, cacheCreate: 0, output: 0 };
      void updateContext(0);
      el<HTMLButtonElement>('promote-run').hidden = true;
      document.querySelectorAll('[data-session-id].is-active').forEach((node) => node.classList.remove('is-active'));
      input.focus();
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

  const action = target.closest<HTMLElement>('[data-action]')?.dataset.action;
  if (action === 'add-context-repo') {
    event.stopPropagation();
    void addContextRepo();
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

async function loadHarnessData(workspaceOverride?: string): Promise<void> {
  try {
    let savedWorkspace: string | null = null;
    try {
      savedWorkspace = localStorage.getItem('harness.workspace');
    } catch {
      // Use the configured default when local storage is unavailable.
    }
    const workspace = workspaceOverride ?? savedWorkspace ?? await bridge.workspace();
    currentWorkspace = workspace;
    renderStaticRegions(workspace);
    const connection = el('connection-state');
    connection.textContent = 'Harness conectado';
    void loadSettings(workspace);
    const sessions = await bridge.sessions(workspace);
    if (sessions[0]) await loadSession(workspace, sessions[0].session_id);
    renderSessionLists(sessions);
    const parent = workspace.replace(/\/+$/, '').split('/').slice(0, -1).join('/') || '/';
    currentProjects = await bridge.projects(parent);
    renderWorkspaceGroups(currentProjects);
    el('project-chip').firstChild!.textContent = workspace.replace(/\/+$/, '').split('/').pop() || workspace;
    if (!el<HTMLElement>('artifacts-view').hidden) void loadArtifactCatalog();
  } catch (error) {
    el('connection-state').textContent = 'Harness offline';
    showNotice('Harness indisponível', error instanceof Error ? error.message : String(error), true);
    renderStaticRegions('Workspace não selecionado');
  }
}

void loadHarnessData();
void updateContext(0);
setPlaceholder();
input.focus();
