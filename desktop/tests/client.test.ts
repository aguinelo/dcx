import assert from 'node:assert/strict';
import test from 'node:test';
import { DcxClient } from '../src/dcx/client.ts';

function eventStream(records: string[]): Response {
  const encoder = new TextEncoder();
  return new Response(
    new ReadableStream({
      start(controller) {
        for (const record of records) controller.enqueue(encoder.encode(record));
        controller.close();
      },
    }),
  );
}

test('run events reconnect from the last delivered event', async () => {
  const urls: string[] = [];
  let calls = 0;
  const client = new DcxClient({
    baseUrl: 'http://dcx.test',
    fetchImpl: async (input) => {
      urls.push(String(input));
      calls += 1;
      if (calls === 1) return eventStream(['event: running\ndata: {}\n\n']);
      return eventStream(['event: done\ndata: {}\n\n']);
    },
  });

  const events: string[] = [];
  await client.events('workspace', 'run-1', (event) => events.push(event.event));

  assert.deepEqual(events, ['running', 'done']);
  assert.match(urls[0] ?? '', /since=0/);
  assert.match(urls[1] ?? '', /since=1/);
});

test('loops are listed, paused and resumed through the daemon', async () => {
  const requests: Array<{ url: string; method: string; body: string | undefined }> = [];
  const client = new DcxClient({
    baseUrl: 'http://dcx.test',
    fetchImpl: async (input, init) => {
      requests.push({ url: String(input), method: init?.method ?? 'GET', body: init?.body as string | undefined });
      return new Response(JSON.stringify({ loops: [], status: 'ok' }), { status: 200 });
    },
  });

  await client.loops('/ws/proj');
  await client.pauseLoop('/ws/proj', 'loop-1');
  await client.resumeLoop('/ws/proj', 'loop-1');

  assert.equal(requests[0]?.url, 'http://dcx.test/loops?workspace=%2Fws%2Fproj');
  assert.equal(requests[1]?.method, 'POST');
  assert.equal(requests[1]?.url, 'http://dcx.test/loops/pause');
  assert.deepEqual(JSON.parse(requests[1]?.body ?? '{}'), { workspace: '/ws/proj', id: 'loop-1' });
  assert.equal(requests[2]?.url, 'http://dcx.test/loops/resume');
});

test('a refused resume surfaces the daemon error message', async () => {
  const client = new DcxClient({
    baseUrl: 'http://dcx.test',
    fetchImpl: async () => new Response(JSON.stringify({ error: 'a fila já está em execução' }), { status: 409 }),
  });

  await assert.rejects(client.resumeLoop('/ws', 'loop-1'), /já está em execução/);
});
