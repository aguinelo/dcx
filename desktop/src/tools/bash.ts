import { z } from 'zod';
import type { Tool } from './types.ts';

const BashInput = z.object({
  command: z.string().min(1),
  /** Per-call timeout in milliseconds. */
  timeout: z.number().int().positive().optional(),
});

/**
 * Run a command in the session's persistent shell.
 *
 * Not a fresh `exec` per call: `cwd` and exported variables survive, which is
 * what makes `cd` then `ls` behave the way a person expects.
 */
export const bashTool: Tool<typeof BashInput> = {
  name: 'bash',
  description:
    'Run a bash command in a persistent shell. State (cwd, env vars) persists across calls.',
  effect: 'exec',
  schema: BashInput,

  async run(input, ctx) {
    const result = await ctx.shell.run(input.command, {
      ...(input.timeout !== undefined ? { timeoutMs: input.timeout } : {}),
    });

    const body = result.stdout.trim();
    if (result.timedOut) {
      return `[command timed out]\n${body}`;
    }
    if (result.exitCode === 0) return body;

    return `[exit ${result.exitCode}]\n${body}`;
  },
};
