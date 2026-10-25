import type { Tool, ToolEffect } from '../tools/types.ts';

export type PermissionMode = 'manual' | 'acceptEdits' | 'plan' | 'deny';

export type PermissionDecision =
  | { outcome: 'allow' }
  | { outcome: 'deny'; reason: string }
  | { outcome: 'ask'; reason: string };

export interface PermissionRequest {
  tool: string;
  effect: ToolEffect;
  input: unknown;
  reason: string;
}

/** `"Bash(git *)"` -> tool `bash` scoped to commands matching `git *`. */
interface CompiledRule {
  tool: string;
  pattern: RegExp | undefined;
  raw: string;
}

const RULE_PATTERN = /^([A-Za-z_][A-Za-z0-9_]*)(?:\((.*)\))?$/;

/** Field of a tool's input a rule pattern applies to, per tool. */
const PATTERN_FIELD: Record<string, string> = {
  bash: 'command',
};

function compileRule(raw: string): CompiledRule {
  const match = RULE_PATTERN.exec(raw.trim());
  if (!match) return { tool: raw.trim().toLowerCase(), pattern: undefined, raw };

  const [, tool, body] = match;
  const name = (tool as string).toLowerCase();
  if (body === undefined) return { tool: name, pattern: undefined, raw };

  // Claude-style wildcards: `*` any run, `:*` prefix match.
  const source = body
    .trim()
    .split(':')
    .map((part) => part.trim())
    .filter(Boolean)
    .map((part) =>
      part === '*' ? '.*' : part.replace(/[.+?^${}()|[\]\\]/g, '\\$&').replace(/\*/g, '.*'),
    )
    .join('|');

  return { tool: name, pattern: new RegExp(`^(?:${source})$`, 'i'), raw };
}

function subjectFor(tool: string, input: unknown): string {
  const field = PATTERN_FIELD[tool];
  if (!field) return '';
  if (typeof input !== 'object' || input === null) return '';
  const value = (input as Record<string, unknown>)[field];
  return typeof value === 'string' ? value : '';
}

function ruleMatches(rule: CompiledRule, tool: string, input: unknown): boolean {
  if (rule.tool !== tool.toLowerCase()) return false;
  if (!rule.pattern) return true;
  return rule.pattern.test(subjectFor(tool, input));
}

export interface PermissionGateOptions {
  mode: PermissionMode;
  allow?: string[];
  deny?: string[];
  /** Answers prompts. Absent means nothing can be approved. */
  ask?: (request: PermissionRequest) => Promise<boolean>;
}

/**
 * Layered permission check.
 *
 * Order is deliberate: explicit deny beats everything, then explicit allow,
 * then mode defaults, and only what is left over reaches the human.
 */
export class PermissionGate {
  private readonly allow: CompiledRule[];
  private readonly deny: CompiledRule[];

  constructor(private readonly options: PermissionGateOptions) {
    this.allow = (options.allow ?? []).map(compileRule);
    this.deny = (options.deny ?? []).map(compileRule);
  }

  /** Decide without prompting. Used by tests and by `deny` mode. */
  evaluate(tool: Tool, input: unknown): PermissionDecision {
    const denied = this.deny.find((rule) => ruleMatches(rule, tool.name, input));
    if (denied) return { outcome: 'deny', reason: `matched deny rule: ${denied.raw}` };

    const allowed = this.allow.find((rule) => ruleMatches(rule, tool.name, input));
    if (allowed) return { outcome: 'allow' };

    const mode = this.options.mode;
    if (mode === 'deny') return { outcome: 'deny', reason: 'permission mode is deny' };

    if (tool.effect === 'read') return { outcome: 'allow' };

    if (mode === 'plan') {
      return { outcome: 'deny', reason: 'plan mode is read-only; no changes may be made' };
    }

    if (mode === 'acceptEdits' && tool.effect === 'write') return { outcome: 'allow' };

    return { outcome: 'ask', reason: `${tool.name} is a ${tool.effect} operation` };
  }

  /** Evaluate, escalating to the human only when the rules are silent. */
  async check(tool: Tool, input: unknown): Promise<PermissionDecision> {
    const decision = this.evaluate(tool, input);
    if (decision.outcome !== 'ask') return decision;

    if (!this.options.ask) {
      return { outcome: 'deny', reason: `${decision.reason} and no approver is configured` };
    }

    const approved = await this.options.ask({
      tool: tool.name,
      effect: tool.effect,
      input,
      reason: decision.reason,
    });

    return approved
      ? { outcome: 'allow' }
      : { outcome: 'deny', reason: `${decision.reason} and the approver said no` };
  }
}
