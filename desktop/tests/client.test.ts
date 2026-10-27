import assert from 'node:assert/strict';
import test from 'node:test';
import { HarnessClient } from '../src/harness/client.ts';

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
  const client = new HarnessClient({
    baseUrl: 'http://harness.test',
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
