import assert from 'node:assert/strict';
import { mkdtempSync } from 'node:fs';
import { tmpdir } from 'node:os';
import path from 'node:path';
import { describe, it } from 'node:test';
import { z } from 'zod';
import { type AgentEvent, runAgent } from '../src/agent/loop.ts';
import { PermissionGate } from '../src/agent/permissions.ts';
import type {
  ContentBlock,
  CreateMessageRequest,
  CreateMessageResponse,
  Provider,
} from '../src/provider/types.ts';
import { ToolRegistry } from '../src/tools/registry.ts';
import { PersistentShell } from '../src/tools/shell.ts';
import { type Tool, type ToolContext, ToolError } from '../src/tools/types.ts';

const workdir = mkdtempSync(path.join(tmpdir(), 'repl-loop-'));

function makeContext(): { context: ToolContext; shell: PersistentShell } {
  const shell = new PersistentShell(workdir);
  return { context: { sandbox: { workdir }, shell }, shell };
}

function text(value: string): ContentBlock {
  return { type: 'text', text: value };
}

function toolUse(id: string, name: string, input: unknown): ContentBlock {
  return { type: 'tool_use', id, name, input };
}

/** Provider that replays a fixed script and records what it was asked. */
function scriptedProvider(
  script: ContentBlock[][],
): Provider & { requests: CreateMessageRequest[] } {
  const requests: CreateMessageRequest[] = [];
  let index = 0;

  return {
    name: 'scripted',
    requests,
    async createMessage(request: CreateMessageRequest): Promise<CreateMessageResponse> {
      requests.push(request);
      const content = script[index] ?? [text('done')];
      index += 1;
      return {
        content,
        stopReason: 'tool_use',
        usage: { inputTokens: 10, outputTokens: 5 },
      };
    },
  };
}

function echoTool(name: string, fail = false): Tool {
  return {
    name,
    effect: 'read',
    description: 'echo',
    schema: z.object({ value: z.string() }),
    run: async (input) => {
      if (fail) throw new ToolError(`${name} exploded`);
      return `echo:${input.value}`;
    },
  };
}

const BASE = {
  systemPrompt: 'system',
  model: 'test-model',
  prompt: 'go',
};

describe('runAgent', () => {
  it('returns the final text when the model never calls a tool', async () => {
    const provider = scriptedProvider([[text('all done')]]);
    const { context, shell } = makeContext();

    try {
      const result = await runAgent({
        ...BASE,
        provider,
        registry: new ToolRegistry([echoTool('echo')]),
        gate: new PermissionGate({ mode: 'manual' }),
        context,
      });

      assert.equal(result.text, 'all done');
      assert.equal(result.inputTokens, 10);
      assert.equal(result.outputTokens, 5);
    } finally {
      shell.close();
    }
  });

  it('executes a tool and feeds the result back to the model', async () => {
    const provider = scriptedProvider([
      [toolUse('t1', 'echo', { value: 'hi' })],
      [text('finished')],
    ]);
    const { context, shell } = makeContext();

    try {
      const result = await runAgent({
        ...BASE,
        provider,
        registry: new ToolRegistry([echoTool('echo')]),
        gate: new PermissionGate({ mode: 'manual' }),
        context,
      });

      assert.equal(result.text, 'finished');

      // The second request must carry the assistant tool_use and a matching result.
      const second = provider.requests[1];
      assert.ok(second);
      const assistant = second.messages.find((m) => m.role === 'assistant');
      const followUp = second.messages.find(
        (m) => m.role === 'user' && m.content.some((b) => b.type === 'tool_result'),
      );
      assert.ok(assistant?.content.some((b) => b.type === 'tool_use'));
      const resultBlock = followUp?.content.find((b) => b.type === 'tool_result');
      assert.equal(resultBlock?.type === 'tool_result' ? resultBlock.content : null, 'echo:hi');
    } finally {
      shell.close();
    }
  });

  it('returns an error result for an unknown tool and keeps going', async () => {
    const provider = scriptedProvider([[toolUse('t1', 'nope', {})], [text('recovered')]]);
    const { context, shell } = makeContext();

    try {
      const result = await runAgent({
        ...BASE,
        provider,
        registry: new ToolRegistry([echoTool('echo')]),
        gate: new PermissionGate({ mode: 'manual' }),
        context,
      });

      assert.equal(result.text, 'recovered');
      const second = provider.requests[1];
      const block = second?.messages.at(-1)?.content.find((b) => b.type === 'tool_result');
      assert.ok(block && block.type === 'tool_result');
      assert.equal(block.is_error, true);
      assert.match(block.content, /unknown tool/);
    } finally {
      shell.close();
    }
  });

  it('surfaces a permission denial as an error result', async () => {
    const provider = scriptedProvider([[toolUse('t1', 'echo', { value: 'x' })], [text('stopped')]]);
    const { context, shell } = makeContext();
    const events: AgentEvent[] = [];

    try {
      await runAgent({
        ...BASE,
        provider,
        registry: new ToolRegistry([echoTool('echo')]),
        gate: new PermissionGate({ mode: 'deny' }),
        context,
        onEvent: (event) => events.push(event),
      });

      assert.ok(events.some((e) => e.type === 'denied'));
      const block = provider.requests[1]?.messages
        .at(-1)
        ?.content.find((b) => b.type === 'tool_result');
      assert.ok(block && block.type === 'tool_result');
      assert.equal(block.is_error, true);
      assert.match(block.content, /permission denied/);
    } finally {
      shell.close();
    }
  });

  it('rejects input that fails schema validation', async () => {
    const provider = scriptedProvider([[toolUse('t1', 'echo', { value: 42 })], [text('ok')]]);
    const { context, shell } = makeContext();

    try {
      await runAgent({
        ...BASE,
        provider,
        registry: new ToolRegistry([echoTool('echo')]),
        gate: new PermissionGate({ mode: 'manual' }),
        context,
      });

      const block = provider.requests[1]?.messages
        .at(-1)
        ?.content.find((b) => b.type === 'tool_result');
      assert.ok(block && block.type === 'tool_result');
      assert.match(block.content, /invalid input/);
    } finally {
      shell.close();
    }
  });

  it('converts a thrown ToolError into an error result instead of crashing', async () => {
    const provider = scriptedProvider([[toolUse('t1', 'echo', { value: 'x' })], [text('ok')]]);
    const { context, shell } = makeContext();

    try {
      const result = await runAgent({
        ...BASE,
        provider,
        registry: new ToolRegistry([echoTool('echo', true)]),
        gate: new PermissionGate({ mode: 'manual' }),
        context,
      });

      assert.equal(result.text, 'ok');
      const block = provider.requests[1]?.messages
        .at(-1)
        ?.content.find((b) => b.type === 'tool_result');
      assert.ok(block && block.type === 'tool_result');
      assert.match(block.content, /exploded/);
    } finally {
      shell.close();
    }
  });

  it('runs several tool calls in one turn', async () => {
    const provider = scriptedProvider([
      [toolUse('t1', 'echo', { value: 'a' }), toolUse('t2', 'echo', { value: 'b' })],
      [text('both done')],
    ]);
    const { context, shell } = makeContext();

    try {
      await runAgent({
        ...BASE,
        provider,
        registry: new ToolRegistry([echoTool('echo')]),
        gate: new PermissionGate({ mode: 'manual' }),
        context,
      });

      const results = provider.requests[1]?.messages
        .at(-1)
        ?.content.filter(
          (b): b is Extract<ContentBlock, { type: 'tool_result' }> => b.type === 'tool_result',
        );
      assert.equal(results?.length, 2);
      assert.deepEqual(
        results?.map((r) => r.content),
        ['echo:a', 'echo:b'],
      );
    } finally {
      shell.close();
    }
  });

  it('stops at maxTurns even if the model keeps asking for tools', async () => {
    const provider = scriptedProvider(
      Array.from({ length: 20 }, () => [toolUse('t', 'echo', { value: 'loop' })]),
    );
    const { context, shell } = makeContext();

    try {
      const result = await runAgent({
        ...BASE,
        provider,
        registry: new ToolRegistry([echoTool('echo')]),
        gate: new PermissionGate({ mode: 'manual' }),
        context,
        maxTurns: 3,
      });

      assert.equal(provider.requests.length, 3);
      assert.equal(result.turns, 3);
    } finally {
      shell.close();
    }
  });

  it('sends the tool specs to the provider', async () => {
    const provider = scriptedProvider([[text('ok')]]);
    const { context, shell } = makeContext();

    try {
      await runAgent({
        ...BASE,
        provider,
        registry: new ToolRegistry([echoTool('echo')]),
        gate: new PermissionGate({ mode: 'manual' }),
        context,
      });

      const first: CreateMessageRequest | undefined = provider.requests[0];
      assert.equal(first?.tools[0]?.name, 'echo');
      assert.equal(first?.tools[0]?.input_schema.type, 'object');
    } finally {
      shell.close();
    }
  });

  it('keeps the prompt as the first user message', async () => {
    const provider = scriptedProvider([[text('ok')]]);
    const { context, shell } = makeContext();

    try {
      await runAgent({
        ...BASE,
        provider,
        registry: new ToolRegistry([echoTool('echo')]),
        gate: new PermissionGate({ mode: 'manual' }),
        context,
      });

      const first = provider.requests[0]?.messages[0];
      assert.deepEqual(first, { role: 'user', content: [text('go')] });
    } finally {
      shell.close();
    }
  });
});
