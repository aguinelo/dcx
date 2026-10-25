/**
 * Renderer.
 *
 * Holds no agent logic. It renders events that arrive over the preload bridge
 * and sends prompts back. The only reason this file exists is the DOM.
 */

import {
  ALL_CONVERSATIONS,
  BACKGROUND_COMPLETED,
  BACKGROUND_TASKS,
  type BackgroundTask,
  CHANGES,
  CONNECTORS_STALE,
  CONTEXT_BREAKDOWN,
  CONTEXT_USED,
  CURRENT_BRANCH,
  type ChangeItem,
  type ContextSlice,
  type Conversation,
  ENVIRONMENT,
  MEMORY_FILES,
  PINNED,
  PROJECT_GROUPS,
  type ProjectGroup,
  TERMINAL_SESSIONS,
  TRANSCRIPT,
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
  type: 'text' | 'tool_start' | 'tool_end' | 'denied' | 'done' | 'error';
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
}

interface ReplBridge {
  run(prompt: string): Promise<void>;
  cancel(): Promise<boolean>;
  context(usedTokens: number): Promise<{ used: number; limit: number; percent: number }>;
  onEvent(fn: (payload: AgentEventPayload) => void): () => void;
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

/* ── Sidebar and panel content ────────────────────────────────── */

function conversationNode(item: Conversation): HTMLElement {
  const node = document.createElement('li');
  if (item.active) node.classList.add('is-active');

  const glyph = document.createElement('span');
  glyph.className = 'glyph';
  glyph.textContent = '</>';

  const title = document.createElement('span');
  title.className = 'title';
  title.textContent = item.title;

  node.append(glyph, title);

  if (item.warning) {
    const warn = document.createElement('span');
    warn.className = 'warn';
    warn.textContent = '⚠';
    node.append(warn);
  }
  if (item.unread) {
    const dot = document.createElement('span');
    dot.className = 'unread';
    node.append(dot);
  }
  return node;
}

function renderPinned(): void {
  el<HTMLUListElement>('pinned').replaceChildren(...PINNED.map(conversationNode));
}

function renderAllConversations(): void {
  el<HTMLUListElement>('all-conversations').replaceChildren(
    ...ALL_CONVERSATIONS.map(conversationNode),
  );
}

function projectSection(group: ProjectGroup): HTMLElement {
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

function renderProjects(): void {
  el('groups').replaceChildren(...PROJECT_GROUPS.map(projectSection));
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

function renderEnvironment(): void {
  const rows = ENVIRONMENT.map((item, index) => envNode(item.glyph, item.label, index === 0));
  rows.push(envNode('⑂', CURRENT_BRANCH));
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

function breakdownNode(slice: ContextSlice): HTMLElement {
  const node = document.createElement('li');

  const swatch = document.createElement('span');
  swatch.className = 'swatch';
  swatch.style.background = slice.color;

  const label = document.createElement('span');
  label.className = 'label';
  label.textContent = slice.label;

  const tokens = document.createElement('span');
  tokens.className = 'tokens';
  tokens.textContent = formatTokens(slice.tokens);

  const percent = document.createElement('span');
  percent.className = 'pct';
  percent.textContent = slice.percent;

  node.append(swatch, label, tokens, percent);
  return node;
}

function renderBreakdown(): void {
  const nodes = CONTEXT_BREAKDOWN.map(breakdownNode);

  const memory = document.createElement('li');
  const memoryLabel = document.createElement('span');
  memoryLabel.className = 'label';
  memoryLabel.textContent = 'Arquivos de memória';
  const memoryCount = document.createElement('span');
  memoryCount.className = 'pct';
  memoryCount.textContent = String(MEMORY_FILES);
  memory.append(memoryLabel, memoryCount);

  el<HTMLUListElement>('breakdown').replaceChildren(...nodes, memory);
}

function formatCount(value: number): string {
  if (value < 1000) return String(value);
  return `${(value / 1000).toFixed(1).replace('.', ',')} mil`;
}

function changeNode(item: ChangeItem): HTMLElement {
  const node = document.createElement('li');
  node.className = 'change';

  const title = document.createElement('div');
  title.className = 'change__title';
  title.append(document.createTextNode(item.title));
  if (item.ready) {
    const dot = document.createElement('span');
    dot.className = 'change__status';
    title.append(dot);
  }

  const repo = document.createElement('div');
  repo.className = 'change__repo';
  repo.textContent = `${item.repo} #${item.number}`;

  const stat = document.createElement('div');
  stat.className = 'change__stat';

  const added = document.createElement('span');
  added.className = 'change__add';
  added.textContent = `+${formatCount(item.additions)}`;

  const removed = document.createElement('span');
  removed.className = 'change__del';
  removed.textContent = `−${formatCount(item.deletions)}`;

  const files = document.createElement('span');
  files.className = 'muted';
  files.textContent = `${item.files} arquivos`;

  stat.append(added, removed, files);

  const action = document.createElement('button');
  action.className = item.ready ? 'btn btn--done' : 'btn';
  action.textContent = item.ready ? 'Pronto' : 'Marcar como pronto';
  action.disabled = item.ready;

  node.append(title, repo, stat, action);
  return node;
}

function renderPanel(): void {
  el('connectors').textContent = `${CONNECTORS_STALE} para reconectar`;
  renderBreakdown();
  el<HTMLUListElement>('changes').replaceChildren(...CHANGES.map(changeNode));
}

function renderStaticRegions(): void {
  renderAllConversations();
  renderPinned();
  renderProjects();
  renderEnvironment();
  renderPanel();
  renderTerminal('repl');
  renderBackground();
  startBackgroundTicker();
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
  { id: 'light', label: 'Claro' },
  { id: 'oled', label: 'OLED' },
  { id: 'dusk', label: 'Anoitecer' },
  { id: 'sepia', label: 'Sépia' },
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

/* ── Background tasks ────────────────────────────────────────── */

/* Cloned so the mock stays a seed rather than mutable render state. */
const backgroundTasks: BackgroundTask[] = BACKGROUND_TASKS.map((task) => ({ ...task }));
let backgroundCompleted = BACKGROUND_COMPLETED;

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

/** The counter is what sells "still running" — it has to actually move. */
function startBackgroundTicker(): void {
  window.setInterval(() => {
    let moved = false;
    for (const task of backgroundTasks) {
      if (!task.running) continue;
      task.elapsed += 1;
      moved = true;
    }
    if (moved) renderBackground();
  }, 1000);
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
  return (
    text
      .split('\n')
      .find((line) => line.trim() !== '')
      ?.trim() ?? ''
  );
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
  thread.append(row('msg msg--user', `<div class="msg__body">${escapeHtml(text)}</div>`));
  scrollToEnd();
}

function addAssistantMessage(text: string): void {
  thread.append(row('msg', `<div class="msg__body">${blockMarkup(text)}</div>`));
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

/** Opens on a real conversation so the thread is never empty on launch. */
function seedTranscript(): void {
  for (const item of TRANSCRIPT) thread.append(transcriptNode(item));
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
  const node = row(
    'activity activity--running',
    `<span class="activity__glyph">◆</span><span>${escapeHtml(name)}</span>${detail ? `<span class="activity__detail">${escapeHtml(detail)}</span>` : ''}`,
  );
  node.dataset.id = id;
  thread.append(node);
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

let accumulatedTokens = CONTEXT_USED;

bridge.onEvent((payload) => {
  switch (payload.type) {
    case 'tool_start':
      if (payload.id) addActivity(payload.id, payload.name ?? 'tool', summarize(payload.input));
      break;

    case 'tool_end':
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
      accumulatedTokens += (payload.inputTokens ?? 0) + (payload.outputTokens ?? 0);
      void updateContext(accumulatedTokens);
      busy = false;
      setPlaceholder();
      break;
    }

    case 'error':
      showNotice('Falha ao falar com o modelo', payload.message ?? 'erro desconhecido', true);
      busy = false;
      setPlaceholder();
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
  input.placeholder = busy ? 'O agente está trabalhando…' : 'Digite / para comandos';
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
    case 'new':
      thread.replaceChildren();
      accumulatedTokens = 0;
      void updateContext(0);
      input.focus();
      return;
    default:
  }
}

document.addEventListener('click', (event) => {
  const target = event.target as HTMLElement;

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

renderStaticRegions();
seedTranscript();
void updateContext(CONTEXT_USED);
setPlaceholder();
input.focus();
