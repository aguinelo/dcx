import type {
  ContentBlock,
  CreateMessageRequest,
  CreateMessageResponse,
  Provider,
} from '../provider/types.ts';

/**
 * Provider that replays a fixed script of assistant turns.
 *
 * Used to exercise the loop and the UI without a live API key. Each script
 * entry is what the model would return for that turn; the final entry should be
 * narration with no tool calls.
 */
export function createScriptedProvider(script: ContentBlock[][]): Provider {
  let index = 0;

  return {
    name: 'scripted',
    async createMessage(_request: CreateMessageRequest): Promise<CreateMessageResponse> {
      const content = script[index] ?? [{ type: 'text', text: 'nothing left to do' }];
      index += 1;
      return {
        content,
        stopReason: 'end_turn',
        usage: { inputTokens: 42, outputTokens: 17 },
      };
    },
  };
}

/** Script covering a realistic turn: read, write, bash, a refusal, then an answer. */
export const DEMO_SCRIPT: ContentBlock[][] = [
  [{ type: 'tool_use', id: 't1', name: 'read', input: { file_path: 'calc.js' } }],
  [
    {
      type: 'tool_use',
      id: 't2',
      name: 'write',
      input: { file_path: 'currency.js', content: 'export const formatBRL = …\n' },
    },
  ],
  [{ type: 'tool_use', id: 't3', name: 'bash', input: { command: 'ls -la' } }],
  [{ type: 'tool_use', id: 't4', name: 'read', input: { file_path: '../../etc/passwd' } }],
  [{ type: 'text', text: 'Criei currency.js e validei a listagem.' }],
];
