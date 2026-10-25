/**
 * Mock data for the empty states of the shell.
 *
 * Everything the sidebar and the context panel show before the app has real
 * state to show. Kept in one module so replacing it with live data is a single
 * edit rather than a hunt through the renderer.
 */

export interface Conversation {
  title: string;
  warning?: boolean;
  unread?: boolean;
  active?: boolean;
}

export interface ContextSlice {
  label: string;
  tokens: number;
  percent: string;
  color: string;
}

export interface ProjectGroup {
  name: string;
  items: Conversation[];
}

export interface EnvironmentItem {
  glyph: string;
  label: string;
  kind: 'dir' | 'repo';
}

export interface ChangeItem {
  title: string;
  repo: string;
  number: number;
  additions: number;
  deletions: number;
  files: number;
  ready: boolean;
}

/**
 * A turn in the seeded conversation.
 *
 * `assistant` text is rendered with a deliberately tiny markdown subset —
 * bullets, ordered lists, bold, inline code and link-shaped spans. Anything
 * richer belongs in the real markdown renderer, not in the mock.
 *
 * The richer kinds below are the shapes the reference shows mid-session:
 * a question the user has to answer, a command waiting to be run, and the
 * artefacts a turn leaves behind.
 */
export interface QuestionOption {
  label: string;
  /** Keyboard hint shown in a badge, e.g. "⌘↵" on the preselected answer. */
  hint?: string;
  selected?: boolean;
}

export type TranscriptItem =
  | { kind: 'user'; text: string }
  | { kind: 'assistant'; text: string }
  | { kind: 'summary'; text: string }
  | { kind: 'activity'; text: string; tokens?: string; diff?: string }
  | { kind: 'steps'; items: { text: string; state: 'running' | 'done' }[] }
  | { kind: 'question'; text: string; question: string; options: QuestionOption[] }
  | { kind: 'command'; command: string; output?: string; multiline?: boolean }
  | { kind: 'pr'; text: string; number: number }
  | { kind: 'artifact'; badge: string; label: string; meta: string };

export const PINNED: Conversation[] = [
  { title: 'repl — núcleo do agente', active: true, unread: true },
  { title: 'sandbox: por que recusar ".."', warning: true },
  { title: 'shell persistente: geração de pending', unread: true },
  { title: 'RE do Claude Code v2.1.308' },
  { title: 'TUI: capturar pty sem perder frame', warning: true },
  { title: 'electron: guard contra flag de debug' },
  { title: 'permission gate em camadas' },
];

export const PROJECT_GROUPS: ProjectGroup[] = [
  {
    name: 'dados',
    items: [
      { title: 'schema de conversations' },
      { title: 'migração de transcripts', unread: true },
      { title: 'índice de sessions', warning: true },
    ],
  },
  {
    name: 'claudecodeclone ~/workspace',
    items: [
      { title: 'sandbox de filesystem', unread: true },
      { title: 'permission gate em camadas' },
      { title: 'tools: glob e grep', warning: true },
      { title: 'GUI desktop três colunas' },
    ],
  },
  {
    name: 'worktrees',
    items: [{ title: 'validate-session-ids' }, { title: 'fix-flaky-daemon', warning: true }],
  },
];

/**
 * Every conversation, flattened across pinned and project groups.
 *
 * The same title can appear in more than one group, so this dedupes on title
 * and keeps pinned first — the ordering the sidebar shows by default.
 */
export const ALL_CONVERSATIONS: Conversation[] = [
  ...PINNED,
  ...PROJECT_GROUPS.flatMap((group) => group.items),
].filter((item, index, all) => all.findIndex((other) => other.title === item.title) === index);

export const ENVIRONMENT: EnvironmentItem[] = [
  { glyph: '▣', label: 'claudecodeclone', kind: 'dir' },
  { glyph: '▣', label: '~/workspace/dreibox', kind: 'dir' },
  { glyph: '▣', label: 'scratch', kind: 'dir' },
  { glyph: '⑂', label: 'ArcaSolucoes/claudecodeclone', kind: 'repo' },
  { glyph: '⑂', label: 'ArcaSolucoes/eva', kind: 'repo' },
  { glyph: '⑂', label: 'dcode', kind: 'repo' },
];

export const CURRENT_BRANCH = 'feat/desktop-shell';

/**
 * Breakdown of the context window by what is actually occupying it.
 *
 * The reference shows the budget as a ledger, not a single number: messages,
 * tool schemas, skills, memory, system prompt, autocompact buffer, free space.
 * A single percentage hides the fact that tool definitions alone can eat a
 * large slice before a single conversation starts.
 */
export const CONTEXT_BREAKDOWN: ContextSlice[] = [
  { label: 'Messages', tokens: 479_000, percent: '47,9%', color: '#5b8def' },
  { label: 'Tool definitions', tokens: 20_800, percent: '2,1%', color: '#e8834a' },
  { label: 'Skills', tokens: 10_000, percent: '1,0%', color: '#57a773' },
  { label: 'Memory files', tokens: 9_400, percent: '0,9%', color: '#c9a227' },
  { label: 'System prompt', tokens: 2_400, percent: '0,2%', color: '#c96a9e' },
  { label: 'Autocompact buffer', tokens: 33_000, percent: '3,3%', color: '#8a8f98' },
  { label: 'Free space', tokens: 445_400, percent: '44,5%', color: '#3a3a38' },
];

export const MEMORY_FILES = 3;

export const CONNECTORS_STALE = 9;

/**
 * A long-running task that outlives the turn that started it.
 *
 * The reference shows these in their own panel block with a live elapsed
 * counter and a switch that stops them, which is what makes them read as
 * background work rather than more chat.
 */
export interface BackgroundTask {
  id: string;
  glyph: string;
  title: string;
  /** Seconds already on the clock when the panel first renders. */
  elapsed: number;
  running: boolean;
}

export const BACKGROUND_TASKS: BackgroundTask[] = [
  {
    id: 'drive-retry',
    glyph: '>_',
    title: 'Tentar criar o comentário até a API do Drive propagar',
    elapsed: 28,
    running: true,
  },
  {
    id: 'gate-wait',
    glyph: '>_',
    title: 'Esperar o gate do worktree loop-core',
    elapsed: 0,
    running: false,
  },
];

export const BACKGROUND_COMPLETED = 2;

export const CHANGES: ChangeItem[] = [
  {
    title: 'feat(sandbox): recusa de ".." em vez de sanitizar',
    repo: 'claudecodeclone',
    number: 701,
    additions: 4925,
    deletions: 42,
    files: 3,
    ready: false,
  },
  {
    title: 'feat(desktop): casca Electron sobre o loop existente',
    repo: 'claudecodeclone',
    number: 698,
    additions: 1180,
    deletions: 164,
    files: 9,
    ready: true,
  },
  {
    title: 'test(shell): pending por geração sobrevive a restart',
    repo: 'claudecodeclone',
    number: 690,
    additions: 214,
    deletions: 8,
    files: 1,
    ready: true,
  },
  {
    title: 'fix(tools): read reportava "file_path is required"',
    repo: 'claudecodeclone',
    number: 688,
    additions: 12,
    deletions: 11,
    files: 1,
    ready: true,
  },
];

/*
 * The gauge and the ledger below it do not add up in the reference: the bar
 * reads 52% while the listed slices total 55.5%. They are computed from
 * different bases, so the mock mirrors them independently instead of deriving
 * one from the other and producing a total that never appears in the product.
 */
export const CONTEXT_USED = 521_400;

/**
 * The conversation the app opens with.
 *
 * An empty thread hid the two things that most distinguish the reference: the
 * serif reply against the sans user turn, and the collapsed tool summary that
 * sits between them. Seeding real turns means both are visible on launch.
 */
export const TRANSCRIPT: TranscriptItem[] = [
  {
    kind: 'user',
    text: 'Faz o RE completo do Claude Code e me diz como ele funciona por dentro.',
  },
  {
    kind: 'assistant',
    text: [
      'Varri o bundle do Claude Code v2.1.308 e separei o que é harness, o que é loop e o que é policy.',
      '',
      '**O que o loop faz**',
      '',
      '- Monta a janela de contexto a cada turno: system prompt, tools e histórico',
      '- Envia para a Messages API e consome os blocos de `tool_use`',
      '- Executa a tool no sandbox e devolve o `tool_result` no turno seguinte',
      '- Repete até o modelo parar de pedir tools',
      '',
      '**O que é policy, não loop**',
      '',
      '- O permission gate decide antes de executar, e a decisão é por camada',
      '- O sandbox nega `..` em vez de normalizar o caminho — recusar é mais seguro que corrigir',
    ].join('\n'),
  },
  { kind: 'summary', text: 'Executou 4 comandos, usou 1 ferramenta' },
  {
    kind: 'assistant',
    text: [
      'Achei: o guard contra flag de debug é do **main process** e roda antes do renderer subir.',
      'Já escrevi o RE completo e a comparação de TUI.',
    ].join('\n'),
  },
  { kind: 'artifact', badge: 'MD', label: 'CLAUDE-CODE-BASE.md', meta: '624 linhas' },
  { kind: 'artifact', badge: 'MD', label: 'TUI-COMPARISON.md', meta: '318 linhas' },
  {
    kind: 'question',
    text: [
      'O RE ficou em `CLAUDE-CODE-BASE.md`, com 23 seções e a lista de funcionamento.',
      'A comparação de TUI ficou em `TUI-COMPARISON.md`. Nada foi commitado ainda — o',
      'guard de debug aparece em duas versões e diverge entre elas.',
    ].join(' '),
    question: 'Trato o guard como proposital nas duas?',
    options: [
      { label: 'É proposital nas duas', hint: '⌘↵', selected: true },
      { label: 'Só na 2.1.308' },
      { label: 'Compara os dois bundles' },
    ],
  },
  {
    kind: 'user',
    text: 'Compara os dois bundles. E monta o loop com os dois cards já aprovados.',
  },
  {
    kind: 'assistant',
    text: [
      'Monto o loop com os dois cards aprovados (OOT-114 e OOT-115). O OOT-113 e o OOT-116',
      'esperam revisão e entram no próximo, se ele disser sim.',
    ].join('\n'),
  },
  { kind: 'pr', text: 'PR criado', number: 337 },
  {
    kind: 'assistant',
    text: ['O plano está no PR #337. Para rodar o loop, libere o merge:'].join('\n'),
  },
  {
    kind: 'command',
    command:
      'jq \'.permissions.allow += ["Bash(gh pr *)"]\' .claude/settings.local.json > /tmp/s.json && mv /tmp/s.json .claude/settings.local.json',
    output: '(sem saída)',
  },
  {
    kind: 'assistant',
    text: 'E cole na conversa:',
  },
  {
    kind: 'command',
    command:
      '/goal Executar o docs/plano-do-loop-compra-e-fornecedor.md do começo ao fim, item por item, na ordem, seguindo a seção "Como o loop trabalha": começar mergeando o #337; uma feature curta por PR, com a spec primeiro e o teste falhando antes do código; merge só com os seis portões verdes e, no OOT-114, com o antes e o depois medidos na base de validação de 1.000 SKUs e escritos na spec; nada de deploy na produção, nem SSH sem escrita no banco; decisão nova vai para "Decisões pendentes". Termina quando o bun run pronto estiver verde na main, a main estiver publicada no staging e o relatório estiver em .loop-state/relatorio-final.md; no fim, tirar a regra Bash(gh pr *).',
    multiline: true,
  },
  { kind: 'summary', text: 'Executou 2 comandos' },
  {
    kind: 'assistant',
    text: [
      'Achei: a base de conhecimento é o módulo `knowledge` da intranet. Vou ler o formato',
      '(tipos, template) e como se cadastra.',
    ].join('\n'),
  },
  { kind: 'summary', text: 'Executou 3 comandos' },
  {
    kind: 'assistant',
    text: [
      'Vou redigir o documento de política de rebate seguindo o formato padrão da intranet',
      '(Objetivo, Abrangência, Diretrizes, Responsáveis, Consequências, Referências), já que',
      'não há script de importação e o cadastro precisará ser feito manualmente.',
    ].join('\n'),
  },
  {
    kind: 'activity',
    text: 'Criou rebate-taxa-conveniencia.md, executou 1 comando',
    diff: '+137',
  },
  {
    kind: 'question',
    text: [
      'Escrevi o documento do rebate no formato Política da Base de Conhecimento da intranet:',
      'as seis seções obrigatórias conferem, e os metadados de cadastro estão no topo.',
    ].join(' '),
    question: 'Como cadastro na intranet?',
    options: [
      { label: 'Eu colo', hint: '⌘↵', selected: true },
      { label: 'Preencha no navegador' },
      { label: 'Resolver conflitos de merge' },
    ],
  },
  { kind: 'summary', text: 'Executando um comando' },
  {
    kind: 'steps',
    items: [
      { text: 'Trocar rebate_base_cents por price_cents na spec', state: 'running' },
      { text: 'Incluir a fase 0 de verificação das promoções', state: 'running' },
      { text: 'Justificar a multa sobre o original', state: 'running' },
      { text: 'Commitar, push e responder no PR', state: 'running' },
    ],
  },
  {
    kind: 'activity',
    text: 'Executando um comando',
    tokens: '976 tokens',
  },
];

/**
 * Seed for the embedded terminal drawer.
 *
 * Kept as a flat log of lines rather than a real shell: the drawer is a
 * presentation surface here, and the main process owns anything that would
 * actually execute.
 */
export interface TerminalLine {
  kind: 'prompt' | 'output' | 'note';
  text: string;
}

export const TERMINAL_SESSIONS: Record<string, { label: string; lines: TerminalLine[] }> = {
  platform: {
    label: 'platform',
    lines: [
      { kind: 'note', text: '[shell reconnected — replaying buffered output]' },
      { kind: 'prompt', text: '~/workspace/embarca/platform ›' },
      { kind: 'output', text: 'gh pr view 337 --json number,title,reviewDecision' },
      {
        kind: 'output',
        text: '{ "number": 337, "title": "loop: compras e fornecedor", "reviewDecision": "APPROVED" }',
      },
      { kind: 'prompt', text: '~/workspace/embarca/platform ›' },
      { kind: 'output', text: 'bun run pronto' },
      { kind: 'output', text: '✓ 94 pass  ✓ 16 suites  ✓ gate verde' },
      { kind: 'prompt', text: '~/workspace/embarca/platform ›' },
    ],
  },
  repl: {
    label: 'repl',
    lines: [
      { kind: 'prompt', text: '~/workspace/dreibox/claudecodeclone ›' },
      { kind: 'output', text: 'pnpm gate' },
      { kind: 'output', text: 'tests 94 · pass 94 · fail 0' },
      { kind: 'prompt', text: '~/workspace/dreibox/claudecodeclone ›' },
    ],
  },
};
