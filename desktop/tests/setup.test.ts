import test from 'node:test';
import assert from 'node:assert/strict';
import { DcxClient } from '../src/dcx/client.ts';

test('setup returns credential status from the daemon', async () => {
  const calls: Array<{ url: string }> = [];
  const client = new DcxClient({
    baseUrl: 'http://dcx.test',
    fetchImpl: async (input) => {
      calls.push({ url: String(input) });
      return new Response(
        JSON.stringify({
          provider: 'minimax',
          has_minimax_key: true,
          has_anthropic_key: false,
          has_claude_cli: true,
        }),
        { status: 200 },
      );
    },
  });

  const status = await client.setup('/home/user/project');
  assert.equal(calls[0]?.url, 'http://dcx.test/setup?workspace=%2Fhome%2Fuser%2Fproject');
  assert.equal(status.provider, 'minimax');
  assert.equal(status.has_minimax_key, true);
  assert.equal(status.has_anthropic_key, false);
  assert.equal(status.has_claude_cli, true);
});

test('setCredentials posts provider and key to daemon', async () => {
  const requests: Array<{ url: string; body: unknown }> = [];
  const client = new DcxClient({
    baseUrl: 'http://dcx.test',
    fetchImpl: async (input, init) => {
      requests.push({ url: String(input), body: JSON.parse((init?.body as string) ?? '{}') });
      return new Response(
        JSON.stringify({ provider: 'minimax', web_enabled: false, max_tool_calls: 24 }),
        { status: 200 },
      );
    },
  });

  const config = await client.setCredentials('/home/user/project', 'minimax', 'test-key-abc');
  assert.equal(requests[0]?.url, 'http://dcx.test/credentials');
  assert.deepEqual(requests[0]?.body, {
    workspace: '/home/user/project',
    provider: 'minimax',
    api_key: 'test-key-abc',
  });
  assert.equal(config.provider, 'minimax');
});

test('setup with missing keys returns false for has_minimax_key and has_anthropic_key', async () => {
  const client = new DcxClient({
    baseUrl: 'http://dcx.test',
    fetchImpl: async () =>
      new Response(
        JSON.stringify({
          provider: null,
          has_minimax_key: false,
          has_anthropic_key: false,
          has_claude_cli: false,
        }),
        { status: 200 },
      ),
  });

  const status = await client.setup('/home/user/new-project');
  assert.equal(status.has_minimax_key, false);
  assert.equal(status.has_anthropic_key, false);
  assert.equal(status.provider, null);
});
