/**
 * Renderer.
 *
 * Holds no agent logic. It renders events that arrive over the preload bridge
 * and sends prompts back. The only reason this file exists is the DOM.
 */

import { TERMINAL_SESSIONS, type BackgroundTask } from './mock.ts';

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
  run(prompt: string, sessionId?: string): Promise<void>;
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
  authStatus(): Promise<{ loggedIn: boolean; email?: string; orgName?: string; subscriptionType?: string; error?: string }>;
  authLogout(): Promise<{ loggedIn: boolean }>;
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

/* ── Sidebar and panel content ────────────────────────────────── */

function conversationNode(item: { title: string; active?: boolean; sessionId?: string; pinned?: boolean }): HTMLElement {
  const node = document.createElement('li');
  if (item.active) node.classList.add('is-active');

  const glyph = document.createElement('span');
  glyph.className = 'glyph';
  glyph.textContent = '</>';

  const title = document.createElement('span');
  title.className = 'title';
  title.textContent = item.title;

  node.append(glyph, title);

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

function projectSection(group: { name: string; items: Array<{ title: string; active?: boolean }> }): HTMLElement {
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

function renderTerminal(sessionKey: string): void {
  const session = TERMINAL_SESSIONS[sessionKey];
  if (!session) return;

  el('terminal-label').textContent = `Terminal ${session.label}`;
  el('terminal-body').replaceChildren(
    ...session.lines.map((line) => {
      const node = document.createElement('div');
      node.className = `term-line term-line--${line.kind}`;
      node.textContent = line.text;
      return node;
    }),
  );
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
  el('settings-model').textContent = config.model ? `Modelo: ${config.model}` : 'Modelo: via Harness';
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
      showNotice('Falha ao falar com o modelo', payload.message ?? 'erro desconhecido', true);
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
    await bridge.run(prompt, currentSessionId);
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

input.addEventListener('keydown', (event) => {
  if (event.key === 'Enter' && !event.shiftKey) {
    event.preventDefault();
    void submit();
  }
});

/* ── Chrome ──────────────────────────────────────────────────── */

function handleChromeAction(action: string): void {
  switch (action) {
    case 'toggle-sidebar':
      layout.dataset.sidebar = layout.dataset.sidebar === 'hidden' ? 'shown' : 'hidden';
      return;
    case 'toggle-panel':
      layout.dataset.panel = layout.dataset.panel === 'hidden' ? 'shown' : 'hidden';
      return;
    case 'terminal':
      if (layout.dataset.terminal !== 'open') renderTerminal('repl');
      setTerminal(layout.dataset.terminal !== 'open');
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

  const action = target.closest('[data-action]')?.getAttribute('data-action');
  if (action) handleChromeAction(action);
});

async function loadHarnessData(): Promise<void> {
  try {
    const workspace = await bridge.workspace();
    currentWorkspace = workspace;
    renderStaticRegions(workspace);
    const connection = el('connection-state');
    connection.textContent = 'Harness conectado';
    void loadSettings(workspace);
    const sessions = await bridge.sessions(workspace);
    if (sessions[0]) await loadSession(workspace, sessions[0].session_id);
    renderSessionLists(sessions);
    const projects = await bridge.projects(workspace);
    el('groups').replaceChildren(
      projectSection({
        name: 'Workspaces',
        items: projects.map((project) => ({ title: project.path })),
      }),
    );
    el('project-chip').firstChild!.textContent = workspace.replace(/\/+$/, '').split('/').pop() || workspace;
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
