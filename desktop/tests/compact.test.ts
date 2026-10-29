import test from 'node:test';
import assert from 'node:assert/strict';
import { DcxClient } from '../src/dcx/client.ts';

test('compactSession calls POST /sessions/compact', async () => {
  const requests: Array<{ url: string; body: unknown }> = [];
  const client = new DcxClient({
    baseUrl: 'http://dcx.test',
    fetchImpl: async (input, init) => {
      requests.push({ url: String(input), body: JSON.parse((init?.body as string) ?? '{}') });
      return new Response(JSON.stringify({ kept: 15, removed: 5 }), { status: 200 });
    },
  });

  const result = await client.compactSession('/home/user/project', 'sess-abc');
  assert.equal(requests[0]?.url, 'http://dcx.test/sessions/compact');
  assert.deepEqual(requests[0]?.body, { workspace: '/home/user/project', session_id: 'sess-abc' });
  assert.equal(result.kept, 15);
  assert.equal(result.removed, 5);
});
