import type { z } from 'zod';
import type { SandboxOptions } from '../fs/sandbox.ts';
import type { PersistentShell } from './shell.ts';

/** Environment handed to every tool execution. */
export interface ToolContext {
  sandbox: SandboxOptions;
  shell: PersistentShell;
}

/**
 * What a tool does to the world.
 *
 * The permission gate keys off this: `read` tools are auto-allowed by default,
 * `write` and `exec` go through the configured rules.
 */
export type ToolEffect = 'read' | 'write' | 'exec';

export interface Tool<S extends z.ZodTypeAny = z.ZodTypeAny> {
  name: string;
  description: string;
  effect: ToolEffect;
  schema: S;
  run(input: z.infer<S>, ctx: ToolContext): Promise<string>;
}

/** Thrown by tools; the loop turns it into an `is_error` tool_result. */
export class ToolError extends Error {
  constructor(message: string) {
    super(message);
    this.name = 'ToolError';
  }
}
