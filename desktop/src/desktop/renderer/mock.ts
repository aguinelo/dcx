/**
 * Seed data for the two panels that still don't have a real backend behind
 * them (background tasks and the embedded terminal). Everything that has a
 * real data source now (sessions, environment, context, pins) gets it from
 * the Harness daemon instead — see renderer.ts.
 */

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
