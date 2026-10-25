/**
 * Scripted orchestration demo.
 *
 * This is presentation, not agent behaviour: the main process owns the real
 * loop, the sandbox and the tools. What this replays is the *shape* of an
 * orchestrated turn — the main agent splitting work into stages, fanning the
 * stages out across worktrees, each worktree running its own subagents, then
 * merging and closing.
 *
 * `at` is milliseconds from the start of the run. The runner keeps a cursor
 * over this list and applies every event whose time has passed, so the whole
 * script can be paused or cancelled by dropping that cursor.
 */

import type { QuestionOption } from './mock.ts';

export type AgentState = 'queued' | 'running' | 'done' | 'failed';

export interface SimAgent {
  name: string;
  state: AgentState;
  detail: string;
}

export type SimEvent =
  | { at: number; kind: 'assistant'; text: string }
  | { at: number; kind: 'stage'; label: string; state: 'running' | 'done' }
  | { at: number; kind: 'worktree'; path: string; branch: string; agents: SimAgent[] }
  | { at: number; kind: 'agent'; path: string; name: string; state: AgentState; detail: string }
  | { at: number; kind: 'activity'; text: string; diff?: string; tokens?: string }
  | { at: number; kind: 'artifact'; badge: string; label: string; meta: string }
  | { at: number; kind: 'pr'; text: string; number: number }
  | { at: number; kind: 'question'; text: string; question: string; options: QuestionOption[] };

/** The prompt the simulated run is answering. */
export const SIM_PROMPT =
  'Refatora o loop de tools em etapas e roda cada etapa em worktree próprio, em paralelo.';

export const SIMULATION: SimEvent[] = [
  {
    at: 300,
    kind: 'assistant',
    text: [
      'Vou quebrar em quatro etapas. As duas do meio são independentes, então cada uma',
      'roda em um worktree próprio e os subagentes trabalham em paralelo dentro dele.',
      'Só a integração espera as duas terminarem.',
    ].join('\n'),
  },

  /* ── Etapa 1 ───────────────────────────────────────────────── */
  { at: 1400, kind: 'stage', label: 'Etapa 1 · Mapear o terreno', state: 'running' },
  { at: 1700, kind: 'activity', text: 'Executou 2 comandos' },
  {
    at: 2600,
    kind: 'assistant',
    text: [
      'Três pontos de corte: montagem da janela, despacho de `tool_use` e devolução do',
      '`tool_result`. O segundo é o único que segura estado, então é o que vai para',
      'worktree com mais agents.',
    ].join('\n'),
  },
  { at: 3200, kind: 'stage', label: 'Etapa 1 · Mapear o terreno', state: 'done' },

  /* ── Etapa 2 · worktrees em paralelo ──────────────────────── */
  { at: 3400, kind: 'stage', label: 'Etapa 2 · Executar em paralelo', state: 'running' },
  {
    at: 3700,
    kind: 'worktree',
    path: '.claude/worktrees/loop-core',
    branch: 'feat/loop-core',
    agents: [
      { name: 'agent-loop', state: 'running', detail: 'refatorando o despacho' },
      { name: 'test-loop', state: 'queued', detail: 'aguardando' },
    ],
  },
  {
    at: 3900,
    kind: 'worktree',
    path: '.claude/worktrees/gui-shell',
    branch: 'feat/gui-shell',
    agents: [
      { name: 'agent-renderer', state: 'running', detail: 'extraindo o transcript' },
      { name: 'agent-css', state: 'queued', detail: 'aguardando' },
    ],
  },
  {
    at: 4700,
    kind: 'agent',
    path: '.claude/worktrees/loop-core',
    name: 'agent-loop',
    state: 'done',
    detail: '3 pontos de corte isolados',
  },
  {
    at: 4900,
    kind: 'agent',
    path: '.claude/worktrees/loop-core',
    name: 'test-loop',
    state: 'running',
    detail: 'escrevendo 6 testes',
  },
  {
    at: 5600,
    kind: 'agent',
    path: '.claude/worktrees/gui-shell',
    name: 'agent-renderer',
    state: 'done',
    detail: 'transcript separado do chrome',
  },
  {
    at: 5800,
    kind: 'agent',
    path: '.claude/worktrees/gui-shell',
    name: 'agent-css',
    state: 'running',
    detail: 'ajustando tokens',
  },
  {
    at: 6700,
    kind: 'agent',
    path: '.claude/worktrees/loop-core',
    name: 'test-loop',
    state: 'done',
    detail: '6 testes, todos verdes',
  },
  {
    at: 7400,
    kind: 'agent',
    path: '.claude/worktrees/gui-shell',
    name: 'agent-css',
    state: 'done',
    detail: 'tokens alinhados com o claude.ai',
  },
  {
    at: 7900,
    kind: 'assistant',
    text: [
      'Os dois worktrees terminaram sem cruzar fronteira. `loop-core` mexeu em',
      '`src/agent`, `gui-shell` em `src/desktop` — nenhum dos dois tocou o mesmo arquivo.',
    ].join('\n'),
  },
  { at: 8200, kind: 'stage', label: 'Etapa 2 · Executar em paralelo', state: 'done' },

  /* ── Etapa 3 · integração ─────────────────────────────────── */
  { at: 8400, kind: 'stage', label: 'Etapa 3 · Integrar os worktrees', state: 'running' },
  { at: 8800, kind: 'activity', text: 'Executou 1 comando' },
  {
    at: 9400,
    kind: 'assistant',
    text: [
      'Mergeei os dois na main e rodei o gate inteiro antes de seguir — 94 testes,',
      'build e typecheck verdes. Nenhum conflito, porque os worktrees não se cruzavam.',
    ].join('\n'),
  },
  {
    at: 9900,
    kind: 'activity',
    text: 'Mergeou 2 worktrees',
    diff: '+218',
    tokens: '4,1k tokens',
  },
  { at: 10300, kind: 'stage', label: 'Etapa 3 · Integrar os worktrees', state: 'done' },

  /* ── Etapa 4 · fechar ─────────────────────────────────────── */
  { at: 10500, kind: 'stage', label: 'Etapa 4 · Fechar', state: 'running' },
  {
    at: 10900,
    kind: 'artifact',
    badge: 'MD',
    label: 'RELATORIO-loop-refactor.md',
    meta: '4 etapas',
  },
  { at: 11300, kind: 'pr', text: 'PR aberto', number: 702 },
  { at: 11600, kind: 'stage', label: 'Etapa 4 · Fechar', state: 'done' },
  {
    at: 12000,
    kind: 'question',
    text: [
      'As quatro etapas passaram. O PR #702 tem os dois worktrees já mergeados na main,',
      'com o relatório de etapa em `RELATORIO-loop-refactor.md`.',
    ].join(' '),
    question: 'Como fecho?',
    options: [
      { label: 'Deixa aberto pra review', hint: '⌘↵', selected: true },
      { label: 'Mergear agora' },
      { label: 'Abre worktree novo pra etapa 5' },
    ],
  },
];
