import type {
  ContentBlock,
  Message,
  Provider,
  ToolResultBlock,
  ToolUseBlock,
} from '../provider/types.ts';
import type { ToolRegistry } from '../tools/registry.ts';
import { type Tool, type ToolContext, ToolError } from '../tools/types.ts';
import type { PermissionGate } from './permissions.ts';

export type AgentEvent =
  | { type: 'text'; text: string }
  | { type: 'tool_start'; id: string; name: string; input: unknown }
  | { type: 'tool_end'; id: string; name: string; ok: boolean; output: string }
  | { type: 'denied'; id: string; name: string; reason: string };

export interface RunAgentOptions {
  provider: Provider;
  registry: ToolRegistry;
  gate: PermissionGate;
  context: ToolContext;
  systemPrompt: string;
  model: string;
  prompt: string;
  maxTurns?: number;
  maxTokens?: number;
  signal?: AbortSignal;
  onEvent?: (event: AgentEvent) => void;
}

export interface AgentResult {
  text: string;
  turns: number;
  inputTokens: number;
  outputTokens: number;
  stopReason: string;
}

const DEFAULT_MAX_TURNS = 25;

function textOf(blocks: ContentBlock[]): string {
  return blocks
    .filter((block): block is Extract<ContentBlock, { type: 'text' }> => block.type === 'text')
    .map((block) => block.text)
    .join('');
}

function toolUsesOf(blocks: ContentBlock[]): ToolUseBlock[] {
  return blocks.filter((block): block is ToolUseBlock => block.type === 'tool_use');
}

function fail(id: string, content: string): ToolResultBlock {
  return { type: 'tool_result', tool_use_id: id, content, is_error: true };
}

interface CallDeps {
  registry: ToolRegistry;
  gate: PermissionGate;
  context: ToolContext;
  onEvent: ((event: AgentEvent) => void) | undefined;
}

function emit(deps: CallDeps, event: AgentEvent): void {
  deps.onEvent?.(event);
}

/** Resolve the tool a call names, or explain what is available instead. */
function resolveTool(deps: CallDeps, call: ToolUseBlock): Tool | undefined {
  const tool = deps.registry.get(call.name);
  if (!tool) {
    emit(deps, {
      type: 'tool_end',
      id: call.id,
      name: call.name,
      ok: false,
      output: 'unknown tool',
    });
  }
  return tool;
}

/** Permission denial becomes an error result the model can react to. */
async function checkPermission(
  deps: CallDeps,
  call: ToolUseBlock,
  tool: Tool,
): Promise<ToolResultBlock | undefined> {
  const decision = await deps.gate.check(tool, call.input);
  if (decision.outcome !== 'deny') return undefined;

  const message = `permission denied: ${decision.reason}`;
  emit(deps, { type: 'denied', id: call.id, name: call.name, reason: decision.reason });
  emit(deps, { type: 'tool_end', id: call.id, name: call.name, ok: false, output: message });
  return fail(call.id, message);
}

type Validated = { ok: true; data: unknown } | { ok: false; block: ToolResultBlock };

function validate(deps: CallDeps, call: ToolUseBlock, tool: Tool): Validated {
  const parsed = tool.schema.safeParse(call.input);
  if (parsed.success) return { ok: true, data: parsed.data };

  const message = `${call.name}: invalid input: ${parsed.error.issues
    .map((issue) => `${issue.path.join('.') || '(root)'} ${issue.message}`)
    .join('; ')}`;
  emit(deps, { type: 'tool_end', id: call.id, name: call.name, ok: false, output: message });
  return { ok: false, block: fail(call.id, message) };
}

/**
 * Run one tool call and always produce a result block.
 *
 * Every failure path returns an `is_error` result rather than throwing: a model
 * that receives an error can recover, one that receives a missing turn cannot.
 */
async function executeToolCall(call: ToolUseBlock, deps: CallDeps): Promise<ToolResultBlock> {
  emit(deps, { type: 'tool_start', id: call.id, name: call.name, input: call.input });

  const tool = resolveTool(deps, call);
  if (!tool) {
    return fail(
      call.id,
      `unknown tool: ${call.name}. Available: ${deps.registry.names().join(', ')}`,
    );
  }

  const denied = await checkPermission(deps, call, tool);
  if (denied) return denied;

  const validated = validate(deps, call, tool);
  if (!validated.ok) return validated.block;

  try {
    const output = await (tool.run as (input: unknown, ctx: ToolContext) => Promise<string>)(
      validated.data,
      deps.context,
    );
    emit(deps, { type: 'tool_end', id: call.id, name: call.name, ok: true, output });
    return { type: 'tool_result', tool_use_id: call.id, content: output };
  } catch (error) {
    const message = error instanceof ToolError ? error.message : String(error);
    emit(deps, { type: 'tool_end', id: call.id, name: call.name, ok: false, output: message });
    return fail(call.id, message);
  }
}

/**
 * Drive the model/tool cycle until it stops asking for tools.
 *
 * Tool calls in one assistant message run sequentially on purpose: a write
 * followed by a read of the same file depends on ordering.
 */
export async function runAgent(options: RunAgentOptions): Promise<AgentResult> {
  const {
    provider,
    registry,
    gate,
    context,
    systemPrompt,
    model,
    prompt,
    maxTurns = DEFAULT_MAX_TURNS,
    maxTokens = 8192,
    onEvent,
  } = options;

  const deps: CallDeps = { registry, gate, context, onEvent };
  const messages: Message[] = [{ role: 'user', content: [{ type: 'text', text: prompt }] }];

  let inputTokens = 0;
  let outputTokens = 0;
  let finalText = '';
  let stopReason = 'end_turn';
  let turn = 0;

  for (; turn < maxTurns; turn += 1) {
    const response = await provider.createMessage({
      system: systemPrompt,
      messages,
      tools: registry.specs(),
      model,
      maxTokens,
      ...(options.signal ? { signal: options.signal } : {}),
    });

    inputTokens += response.usage.inputTokens;
    outputTokens += response.usage.outputTokens;
    stopReason = response.stopReason;

    const narration = textOf(response.content);
    if (narration) {
      finalText = narration;
      onEvent?.({ type: 'text', text: narration });
    }

    const calls = toolUsesOf(response.content);
    if (calls.length === 0) {
      turn += 1;
      break;
    }

    messages.push({ role: 'assistant', content: response.content });

    const results: ContentBlock[] = [];
    for (const call of calls) {
      results.push(await executeToolCall(call, deps));
    }

    messages.push({ role: 'user', content: results });
  }

  return { text: finalText, turns: turn, inputTokens, outputTokens, stopReason };
}
