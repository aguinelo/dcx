/**
 * Renderer.
 *
 * Holds no agent logic. It renders events that arrive over the preload bridge
 * and sends prompts back. The only reason this file exists is the DOM.
 */

import {
  TERMINAL_SESSIONS,
  type BackgroundTask,
  type TranscriptItem,
} from './mock.ts';
import {
  type AgentState,
  SIMULATION,
  SIM_PROMPT,
  type SimAgent,
  type SimEvent,
} from './simulation.ts';

interface AgentEventPayload {
  type: 'text' | 'progress' | 'tool_start' | 'tool_end' | 'denied' | 'done' | 'error' | 'run_event';
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
}

interface ReplBridge {
  run(prompt: string): Promise<void>;
  cancel(): Promise<boolean>;
  context(usedTokens: number): Promise<{ used: number; limit: number; percent: number }>;
  onEvent(fn: (payload: AgentEventPayload) => void): () => void;
  workspace(): Promise<string>;
  sessions(workspace: string): Promise<Array<{ session_id: string; title: string; message_count: number; updated_at_unix_ms: number }>>;
  session(workspace: string, sessionId: string): Promise<{ messages: Array<{ role: 'user' | 'assistant'; content: string }> }>;
  projects(root: string): Promise<Array<{ path: string }>>;
  promote(workspace: string, sessionId: string): Promise<{ status: string; request: string }>;
  approve(workspace: string, sessionId: string): Promise<unknown>;
  start(workspace: string, sessionId: string): Promise<{ run_id: string; status: string }>;
  config(workspace: string): Promise<{ provider: string; model?: string; web_enabled: boolean; max_tool_calls: number }>;
  setConfig(
    workspace: string,
    updates: Partial<{ provider: string; web_enabled: boolean; max_tool_calls: number }>,
  ): Promise<{ provider: string; model?: string; web_enabled: boolean; max_tool_calls: number }>;
  account(workspace: string): Promise<{ name: string; email?: string }>;
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

function conversationNode(item: { title: string; active?: boolean; sessionId?: string }): HTMLElement {
  const node = document.createElement('li');
  if (item.active) node.classList.add('is-active');

  const glyph = document.createElement('span');
  glyph.className = 'glyph';
  glyph.textContent = '</>';

  const title = document.createElement('span');
  title.className = 'title';
  title.textContent = item.title;

  node.append(glyph, title);
  if (item.sessionId) node.dataset.sessionId = item.sessionId;

  return node;
}

function renderAllConversations(items: Array<{ title: string; active?: boolean; sessionId?: string }>): void {
  el<HTMLUListElement>('all-conversations').replaceChildren(...items.map(conversationNode));
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

function renderEnvironment(workspace: string): void {
  el<HTMLUListElement>('env').replaceChildren(envNode('▣', workspace, true));
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
}

function renderStaticRegions(workspace: string): void {
  renderAllConversations([]);
  el<HTMLUListElement>('pinned').replaceChildren();
  el('groups').replaceChildren();
  renderEnvironment(workspace);
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
const THEMES = [
  { id: 'dark', label: 'Escuro' },
  { id: 'claude', label: 'Claude' },
  { id: 'light', label: 'Claro' },
  { id: 'oled', label: 'OLED' },
  { id: 'dusk', label: 'Anoitecer' },
  { id: 'sepia', label: 'Sépia' },
] as const;

const THEME_KEY = 'repl:theme';
const CUSTOM_THEMES_KEY = 'repl:customThemes';

/** The full token set every built-in [data-theme] block defines in app.css. */
const CUSTOM_THEME_VARS = [
  '--bg',
  '--bg-raised',
  '--bg-hover',
  '--bg-hover-strong',
  '--surface-subtle',
  '--bubble-user',
  '--border',
  '--border-strong',
  '--text',
  '--muted',
  '--faint',
  '--accent',
  '--accent-soft',
  '--ok',
  '--warn',
  '--danger',
  '--code-bg',
  '--hint-bg',
  '--tok-str',
  '--toggle-knob',
  '--scroll-thumb',
  '--scroll-thumb-hover',
] as const;

interface CustomTheme {
  id: string;
  label: string;
  bg: string;
  bgRaised: string;
  text: string;
  border: string;
  accent: string;
}

function hexToRgbTuple(hex: string): [number, number, number] {
  const normalized = hex.replace('#', '');
  const value = Number.parseInt(normalized.length === 3 ? normalized.replace(/(.)/g, '$1$1') : normalized, 16);
  return [(value >> 16) & 255, (value >> 8) & 255, value & 255];
}

function rgbTupleToHex([r, g, b]: [number, number, number]): string {
  return `#${[r, g, b].map((channel) => Math.round(Math.min(255, Math.max(0, channel))).toString(16).padStart(2, '0')).join('')}`;
}

/** Blends two hex colours; `t` of 0 is `hexA`, 1 is `hexB`. */
function mixHex(hexA: string, hexB: string, t: number): string {
  const a = hexToRgbTuple(hexA);
  const b = hexToRgbTuple(hexB);
  return rgbTupleToHex([a[0] + (b[0] - a[0]) * t, a[1] + (b[1] - a[1]) * t, a[2] + (b[2] - a[2]) * t] as [number, number, number]);
}

function relativeLuminance(hex: string): number {
  const [r, g, b] = hexToRgbTuple(hex);
  return (0.2126 * r + 0.7152 * g + 0.0722 * b) / 255;
}

/**
 * A custom theme only asks for 5 colours; every other token is derived from
 * them with the same blend ratios the hand-picked built-in themes land on,
 * so a custom theme reads as consistent rather than hand-tuned per field.
 */
function deriveCustomTokens(theme: CustomTheme): Record<(typeof CUSTOM_THEME_VARS)[number], string> {
  const [tr, tg, tb] = hexToRgbTuple(theme.text);
  const [br, bg, bb] = hexToRgbTuple(theme.bg);
  const textRgb = `${tr} ${tg} ${tb}`;
  const bgRgb = `${br} ${bg} ${bb}`;
  return {
    '--bg': theme.bg,
    '--bg-raised': theme.bgRaised,
    '--bg-hover': mixHex(theme.bg, theme.text, 0.08),
    '--bg-hover-strong': mixHex(theme.bg, theme.text, 0.16),
    '--surface-subtle': `rgb(${textRgb} / 3%)`,
    '--bubble-user': mixHex(theme.bg, theme.text, 0.08),
    '--border': theme.border,
    '--border-strong': mixHex(theme.border, theme.text, 0.3),
    '--text': theme.text,
    '--muted': mixHex(theme.text, theme.bg, 0.4),
    '--faint': mixHex(theme.text, theme.bg, 0.65),
    '--accent': theme.accent,
    '--accent-soft': mixHex(theme.accent, '#000000', 0.2),
    '--ok': '#7f9b7a',
    '--warn': '#c8963e',
    '--danger': '#c9645a',
    '--code-bg': `rgb(${textRgb} / 7%)`,
    '--hint-bg': `rgb(${bgRgb} / 18%)`,
    '--tok-str': '#8fbf7f',
    '--toggle-knob': '#ffffff',
    '--scroll-thumb': `rgb(${textRgb} / 14%)`,
    '--scroll-thumb-hover': `rgb(${textRgb} / 24%)`,
  };
}

function loadCustomThemes(): CustomTheme[] {
  try {
    const raw = localStorage.getItem(CUSTOM_THEMES_KEY);
    return raw ? (JSON.parse(raw) as CustomTheme[]) : [];
  } catch {
    return [];
  }
}

function saveCustomThemes(themes: CustomTheme[]): void {
  try {
    localStorage.setItem(CUSTOM_THEMES_KEY, JSON.stringify(themes));
  } catch {
    // file:// can refuse storage; the themes still apply for this session.
  }
}

let customThemes: CustomTheme[] = loadCustomThemes();
let activeThemeId = 'dark';

function applyTheme(id: string): void {
  const custom = customThemes.find((theme) => theme.id === id);
  const known = custom ? id : THEMES.some((theme) => theme.id === id) ? id : 'dark';
  const root = document.documentElement;

  if (custom) {
    root.dataset.theme = '';
    const tokens = deriveCustomTokens(custom);
    for (const [name, value] of Object.entries(tokens)) root.style.setProperty(name, value);
    root.style.colorScheme = relativeLuminance(custom.bg) > 0.5 ? 'light' : 'dark';
  } else {
    for (const name of CUSTOM_THEME_VARS) root.style.removeProperty(name);
    root.style.removeProperty('color-scheme');
    root.dataset.theme = known;
  }

  for (const child of el('themes').children) {
    child.classList.toggle('is-active', (child as HTMLElement).dataset.theme === known);
  }
  activeThemeId = known;
  try {
    localStorage.setItem(THEME_KEY, known);
  } catch {
    // file:// can refuse storage; the theme still applies for this session.
  }
}

function renderThemes(): void {
  const builtIn = THEMES.map(({ id, label }) => {
    const swatch = document.createElement('button');
    swatch.className = 'theme';
    swatch.dataset.theme = id;
    swatch.title = `Tema ${label}`;
    swatch.setAttribute('role', 'radio');
    swatch.setAttribute('aria-label', `Tema ${label}`);
    return swatch;
  });
  const custom = customThemes.map((theme) => {
    const swatch = document.createElement('button');
    swatch.className = 'theme theme--custom';
    swatch.dataset.theme = theme.id;
    swatch.title = `Tema ${theme.label}`;
    swatch.style.background = `linear-gradient(135deg, ${theme.bg} 45%, ${theme.accent} 45%)`;
    swatch.setAttribute('role', 'radio');
    swatch.setAttribute('aria-label', `Tema ${theme.label}`);
    const remove = document.createElement('span');
    remove.className = 'theme__delete';
    remove.dataset.deleteTheme = theme.id;
    remove.title = `Excluir tema ${theme.label}`;
    remove.textContent = '×';
    swatch.append(remove);
    return swatch;
  });
  el('themes').replaceChildren(...builtIn, ...custom);
}

/* ── Account and provider settings ─────────────────────────────── */

function initials(name: string): string {
  const trimmed = name.trim();
  if (!trimmed) return '?';
  return trimmed.charAt(0).toUpperCase();
}

let currentAccountEmail = '';

function renderAccount(account: { name: string; email?: string }): void {
  const mark = initials(account.name);
  currentAccountEmail = account.email ?? '';
  el('sidebar-avatar').textContent = mark;
  el('sidebar-account-name').textContent = account.name;
  el('settings-avatar').textContent = mark;
  el('settings-account-name').textContent = account.name;
  el('settings-account-email').textContent = currentAccountEmail;
}

/**
 * Prefers the real, shared Claude Code login (same one `claude auth status`
 * reports) over the workspace's git identity, since it is the account that
 * actually authenticates chat turns. Falls back to git when logged out or
 * when `claude` isn't the active provider's CLI.
 */
async function resolveAccountIdentity(workspace: string): Promise<{ name: string; email?: string }> {
  const auth = await bridge.authStatus();
  if (auth.loggedIn && (auth.email || auth.orgName)) {
    const name = auth.orgName ?? auth.email ?? 'Conta Claude';
    return auth.email ? { name, email: auth.email } : { name };
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
}

function addBackgroundTask(glyph: string, title: string): void {
  backgroundTasks.push({
    id: `sim-${backgroundTasks.length}`,
    glyph,
    title,
    elapsed: 0,
    running: true,
  });
  renderBackground();
}

/**
 * Close the tasks this run opened — and only those.
 *
 * The seeded Drive retry is independent of the simulated turn: a stage
 * finishing says nothing about whether an unrelated retry loop stopped, so
 * sweeping every running task here would be a lie.
 */
function completeBackgroundTasks(): void {
  let closed = 0;
  for (const task of backgroundTasks) {
    if (!task.running || !task.id.startsWith('sim-')) continue;
    task.running = false;
    closed += 1;
  }
  if (closed === 0) return;
  backgroundCompleted += closed;
  renderBackground();
}

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

/**
 * Tints a command the way the reference does. Runs on escaped text, so the
 * only strings it can emit are these two spans.
 */
function highlightCommand(escaped: string): string {
  return escaped
    .replace(/(^|[\s(])(&quot;[\s\S]*?&quot;)/g, '$1<span class="tok-str">$2</span>')
    .replace(/^([a-z0-9._/-]+)/i, '<span class="tok-cmd">$1</span>');
}

function commandNode(item: Extract<TranscriptItem, { kind: 'command' }>): HTMLElement {
  const node = row(
    'cmd',
    [
      `<div class="cmd__box">`,
      `<span class="cmd__prompt">&gt;</span>`,
      `<code class="cmd__code">${highlightCommand(escapeHtml(item.command))}</code>`,
      `<span class="cmd__actions">`,
      '<button class="iconbtn iconbtn--tiny" data-cmd-action="run" title="Executar aqui">▷</button>',
      '<button class="iconbtn iconbtn--tiny" data-cmd-action="copy" title="Copiar">⧉</button>',
      '</span>',
      '</div>',
      item.output
        ? `<button class="cmd__output" data-cmd-action="output">${escapeHtml(item.output)}<span class="summary__caret">▸</span></button>`
        : '',
    ].join(''),
  );
  if (item.multiline) node.classList.add('cmd--multiline');
  return node;
}

function questionNode(item: Extract<TranscriptItem, { kind: 'question' }>): HTMLElement {
  const options = item.options
    .map((option) => {
      const hint = option.hint
        ? `<span class="option__hint">${escapeHtml(option.hint)}</span>`
        : '';
      return `<button class="option${option.selected ? ' is-selected' : ''}" data-option>${escapeHtml(option.label)}${hint}</button>`;
    })
    .join('');

  return row(
    'question',
    [
      `<button class="question__close" title="Dispensar">✕</button>`,
      `<div class="question__body">${blockMarkup(item.text)}</div>`,
      `<p class="question__ask">${escapeHtml(item.question)}</p>`,
      `<div class="question__options">${options}</div>`,
    ].join(''),
  );
}

function stepsNode(item: Extract<TranscriptItem, { kind: 'steps' }>): HTMLElement {
  const items = item.items
    .map(
      (entry) =>
        `<li class="steps__item steps__item--${entry.state}"><span class="steps__glyph">${entry.state === 'done' ? '✓' : '◌'}</span><span>${escapeHtml(entry.text)}</span></li>`,
    )
    .join('');
  return row('steps', `<ul class="steps__list">${items}</ul>`);
}

function activityNode(item: Extract<TranscriptItem, { kind: 'activity' }>): HTMLElement {
  const diff = item.diff ? `<span class="activity__diff">${escapeHtml(item.diff)}</span>` : '';
  const tokens = item.tokens
    ? `<span class="activity__detail">${escapeHtml(item.tokens)}</span>`
    : '';
  return row(
    'activity',
    `<span class="activity__glyph">⋯</span><span>${escapeHtml(item.text)}</span>${diff}${tokens}<span class="summary__caret">▸</span>`,
  );
}

function transcriptNode(item: TranscriptItem): HTMLElement {
  switch (item.kind) {
    case 'user':
      return row('msg msg--user', `<div class="msg__body">${escapeHtml(item.text)}</div>`);
    case 'assistant':
      return row('msg', `<div class="msg__body">${blockMarkup(item.text)}</div>`);
    case 'summary':
      return row('summary', `<span class="summary__caret">▸</span>${escapeHtml(item.text)}`);
    case 'question':
      return questionNode(item);
    case 'command':
      return commandNode(item);
    case 'steps':
      return stepsNode(item);
    case 'activity':
      return activityNode(item);
    case 'pr':
      return row(
        'pr',
        `<span>${escapeHtml(item.text)}</span><span class="pr__ref">#${item.number}</span><span class="summary__caret">▸</span>`,
      );
    case 'artifact':
      return row(
        'artifact',
        `<span class="artifact__badge">${escapeHtml(item.badge)}</span><span class="artifact__label">${escapeHtml(item.label)}</span><span class="artifact__meta">${escapeHtml(item.meta)}</span>`,
      );
  }
}

/* ── Orchestrated run ────────────────────────────────────────── */

const AGENT_GLYPH: Record<AgentState, string> = {
  queued: '○',
  running: '◌',
  done: '✓',
  failed: '✕',
};

/** Live handles so later events patch the card they belong to. */
const worktreeNodes = new Map<string, HTMLElement>();
const stageNodes = new Map<string, HTMLElement>();

/**
 * A stage is announced once and resolved in place. Emitting a second header
 * for the same label would show the same stage twice, which is not what the
 * reference does — there the caret just stops pulsing and turns into a tick.
 * Returns null when the stage was already on screen.
 */
function stageNode(label: string, state: 'running' | 'done'): HTMLElement | null {
  const done = state === 'done';
  const existing = stageNodes.get(label);

  if (done && existing) {
    existing.className = 'stage stage--done';
    const glyph = existing.querySelector('.stage__glyph');
    if (glyph) glyph.textContent = '✓';
    return null;
  }

  const node = row(
    'stage',
    `<span class="stage__rule"></span><span class="stage__glyph">${done ? '✓' : '◌'}</span><span class="stage__label">${escapeHtml(label)}</span>`,
  );
  stageNodes.set(label, node);
  return node;
}

/** Each resolved stage costs context, which is what makes the gauge move. */
function bumpContext(): void {
  accumulatedTokens += 121_500;
  void updateContext(accumulatedTokens);
}

function agentRowMarkup(agent: SimAgent): string {
  return [
    `<li class="agent agent--${agent.state}" data-name="${escapeHtml(agent.name)}">`,
    `<span class="agent__glyph">${AGENT_GLYPH[agent.state]}</span>`,
    `<span class="agent__name">${escapeHtml(agent.name)}</span>`,
    `<span class="agent__detail">${escapeHtml(agent.detail)}</span>`,
    '</li>',
  ].join('');
}

function worktreeNode(event: Extract<SimEvent, { kind: 'worktree' }>): HTMLElement {
  const node = row(
    'worktree',
    [
      '<header class="worktree__head">',
      '<span class="worktree__glyph">⑂</span>',
      `<span class="worktree__path">${escapeHtml(event.path)}</span>`,
      `<span class="worktree__branch">${escapeHtml(event.branch)}</span>`,
      '</header>',
      `<ul class="worktree__agents">${event.agents.map(agentRowMarkup).join('')}</ul>`,
    ].join(''),
  );
  worktreeNodes.set(event.path, node);
  return node;
}

/** Patch one subagent row in place rather than rebuilding the card. */
function updateAgent(event: Extract<SimEvent, { kind: 'agent' }>): void {
  const row = worktreeNodes
    .get(event.path)
    ?.querySelector<HTMLElement>(`[data-name="${CSS.escape(event.name)}"]`);
  if (!row) return;

  row.className = `agent agent--${event.state}`;
  const glyph = row.querySelector('.agent__glyph');
  if (glyph) glyph.textContent = AGENT_GLYPH[event.state];
  const detail = row.querySelector('.agent__detail');
  if (detail) detail.textContent = event.detail;
}

function applySimEvent(event: SimEvent): void {
  switch (event.kind) {
    case 'assistant':
      addAssistantMessage(event.text);
      return;
    case 'stage': {
      const node = stageNode(event.label, event.state);
      if (node) thread.append(node);
      if (event.state === 'done') {
        bumpContext();
        completeBackgroundTasks();
      }
      break;
    }
    case 'worktree':
      thread.append(worktreeNode(event));
      addBackgroundTask('⑂', `${event.branch} · ${event.agents.length} subagentes`);
      break;
    case 'agent':
      updateAgent(event);
      return;
    default:
      thread.append(transcriptNode(event as TranscriptItem));
  }
  scrollToEnd();
}

let simTimer: number | undefined;

function stopSimulation(): void {
  if (simTimer === undefined) return;
  window.clearInterval(simTimer);
  simTimer = undefined;
}

function finishSimulation(): void {
  stopSimulation();
  busy = false;
  setPlaceholder();
  el<HTMLButtonElement>('sim-start').hidden = false;
  el<HTMLButtonElement>('sim-stop').hidden = true;
}

function runSimulation(): void {
  if (busy) return;
  stopSimulation();

  busy = true;
  setPlaceholder();
  worktreeNodes.clear();
  stageNodes.clear();
  thread.replaceChildren();
  accumulatedTokens = 0;
  void updateContext(0);
  el<HTMLDivElement>('notice').hidden = true;
  el<HTMLButtonElement>('sim-start').hidden = true;
  el<HTMLButtonElement>('sim-stop').hidden = false;

  addUserMessage(SIM_PROMPT);

  const started = performance.now();
  let cursor = 0;

  simTimer = window.setInterval(() => {
    const elapsed = performance.now() - started;
    while (cursor < SIMULATION.length) {
      const next = SIMULATION[cursor];
      if (!next || next.at > elapsed) break;
      applySimEvent(next);
      cursor += 1;
    }
    if (cursor >= SIMULATION.length) finishSimulation();
  }, 80);
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
}

/* ── Events ──────────────────────────────────────────────────── */

let accumulatedTokens = 0;

bridge.onEvent((payload) => {
  switch (payload.type) {
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
      accumulatedTokens += (payload.inputTokens ?? 0) + (payload.outputTokens ?? 0);
      void updateContext(accumulatedTokens);
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
        busy = false;
        setPlaceholder();
        setThinking(false);
      } else if (payload.event === 'failed' || payload.event === 'cancelled') {
        showNotice('Run encerrado', payload.event, payload.event === 'failed');
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
    await bridge.run(prompt);
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

/** Selecting an answer is a single-choice set, so clear the siblings first. */
function selectOption(chosen: HTMLElement): void {
  const group = chosen.closest('.question__options');
  if (!group) return;
  for (const option of group.querySelectorAll('.option')) {
    option.classList.toggle('is-selected', option === chosen);
  }
}

async function copyCommand(code: HTMLElement): Promise<void> {
  // navigator.clipboard is unavailable on file:// without a permission grant,
  // so fall back to a selection the caller can copy by hand.
  const text = code.textContent ?? '';
  try {
    await navigator.clipboard.writeText(text);
  } catch {
    const range = document.createRange();
    range.selectNodeContents(code);
    const selection = window.getSelection();
    selection?.removeAllRanges();
    selection?.addRange(range);
  }
}

function handleCommandClick(target: HTMLElement): boolean {
  const trigger = target.closest<HTMLElement>('[data-cmd-action]');
  if (!trigger) return false;

  switch (trigger.dataset.cmdAction) {
    case 'run':
      renderTerminal('repl');
      setTerminal(true);
      return true;
    case 'copy': {
      const code = trigger.closest<HTMLElement>('.cmd')?.querySelector<HTMLElement>('.cmd__code');
      if (code) void copyCommand(code);
      return true;
    }
    case 'output':
      trigger.classList.toggle('is-open');
      return true;
    default:
      return false;
  }
}

function handleChromeAction(action: string): void {
  switch (action) {
    case 'toggle-sidebar':
      layout.dataset.sidebar = layout.dataset.sidebar === 'hidden' ? 'shown' : 'hidden';
      return;
    case 'toggle-panel':
      layout.dataset.panel = layout.dataset.panel === 'hidden' ? 'shown' : 'hidden';
      return;
    case 'terminal':
      setTerminal(layout.dataset.terminal !== 'open');
      return;
    case 'toggle-terminal':
      setTerminal(false);
      return;
    case 'toggle-bg':
      el('bg-section').classList.toggle('is-collapsed');
      return;
    case 'simulate':
      runSimulation();
      return;
    case 'stop-sim':
      finishSimulation();
      return;
    case 'settings':
      el('theme-settings').hidden = false;
      void loadSettings(currentWorkspace);
      return;
    case 'close-settings':
      el('theme-settings').hidden = true;
      return;
    case 'new-theme':
      el('theme-form').hidden = false;
      return;
    case 'cancel-theme-form':
      el<HTMLFormElement>('theme-form').hidden = true;
      el<HTMLFormElement>('theme-form').reset();
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
      accumulatedTokens = 0;
      void updateContext(0);
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

el<HTMLFormElement>('theme-form').addEventListener('submit', (event) => {
  event.preventDefault();
  const name = el<HTMLInputElement>('theme-form-name').value.trim();
  if (!name) return;
  const theme: CustomTheme = {
    id: `custom-${Date.now()}`,
    label: name,
    bg: el<HTMLInputElement>('theme-form-bg').value,
    bgRaised: el<HTMLInputElement>('theme-form-bg-raised').value,
    text: el<HTMLInputElement>('theme-form-text').value,
    border: el<HTMLInputElement>('theme-form-border').value,
    accent: el<HTMLInputElement>('theme-form-accent').value,
  };
  customThemes = [...customThemes, theme];
  saveCustomThemes(customThemes);
  renderThemes();
  applyTheme(theme.id);
  el<HTMLFormElement>('theme-form').hidden = true;
  el<HTMLFormElement>('theme-form').reset();
});

document.addEventListener('click', (event) => {
  const target = event.target as HTMLElement;

  const accountMenu = el('account-menu');
  if (!accountMenu.hidden && !accountMenu.contains(target) && !target.closest('[data-action="account-menu"]')) {
    accountMenu.hidden = true;
  }

  const deleteTheme = target.closest<HTMLElement>('[data-delete-theme]');
  if (deleteTheme?.dataset.deleteTheme) {
    event.stopPropagation();
    const id = deleteTheme.dataset.deleteTheme;
    const wasActive = activeThemeId === id;
    customThemes = customThemes.filter((theme) => theme.id !== id);
    saveCustomThemes(customThemes);
    renderThemes();
    if (wasActive) applyTheme('dark');
    return;
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

  const sessionNode = target.closest<HTMLElement>('[data-session-id]');
  if (sessionNode?.dataset.sessionId) {
    void bridge.workspace().then((workspace) => loadSession(workspace, sessionNode.dataset.sessionId!));
    return;
  }

  const option = target.closest<HTMLElement>('[data-option]');
  if (option) {
    selectOption(option);
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

  if (handleCommandClick(target)) return;

  const action = target.closest('[data-action]')?.getAttribute('data-action');
  if (action) handleChromeAction(action);
});

async function loadHarnessData(): Promise<void> {
  try {
    const workspace = await bridge.workspace();
    renderStaticRegions(workspace);
    const connection = el('connection-state');
    connection.textContent = 'Harness conectado';
    void loadSettings(workspace);
    const sessions = await bridge.sessions(workspace);
    renderAllConversations(
      sessions.map((session, index) => ({
        title: session.title || 'Nova conversa',
        active: index === 0,
        sessionId: session.session_id,
      })),
    );
    if (sessions[0]) await loadSession(workspace, sessions[0].session_id);
    const projects = await bridge.projects(workspace);
    el('groups').replaceChildren(
      projectSection({
        name: 'Workspaces',
        items: projects.map((project) => ({ title: project.path })),
      }),
    );
    el('project-chip').firstChild!.textContent = workspace;
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
