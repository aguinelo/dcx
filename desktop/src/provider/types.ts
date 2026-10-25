/**
 * Wire types shared by every provider.
 *
 * These mirror the Anthropic Messages API shape on purpose: it is the reference
 * protocol, and keeping the block taxonomy (`text` / `tool_use` / `tool_result`)
 * makes a future OpenAI or MiniMax adapter a translation layer rather than a
 * rewrite of the agent loop.
 */

export type Role = 'user' | 'assistant';

export interface TextBlock {
  type: 'text';
  text: string;
}

export interface ToolUseBlock {
  type: 'tool_use';
  id: string;
  name: string;
  input: unknown;
}

export interface ToolResultBlock {
  type: 'tool_result';
  tool_use_id: string;
  content: string;
  is_error?: boolean;
}

export type ContentBlock = TextBlock | ToolUseBlock | ToolResultBlock;

/** One turn of the conversation. */
export interface Message {
  role: Role;
  content: ContentBlock[];
}

/** Tool description forwarded to the provider. */
export interface ToolSpec {
  name: string;
  description: string;
  input_schema: {
    type: 'object';
    properties: Record<string, unknown>;
    required?: string[];
  };
}

export interface CreateMessageRequest {
  system: string;
  messages: Message[];
  tools: ToolSpec[];
  model: string;
  maxTokens: number;
  signal?: AbortSignal;
}

export interface CreateMessageResponse {
  content: ContentBlock[];
  stopReason: 'end_turn' | 'tool_use' | 'max_tokens' | 'stop_sequence';
  usage: {
    inputTokens: number;
    outputTokens: number;
  };
}

export interface Provider {
  readonly name: string;
  createMessage(request: CreateMessageRequest): Promise<CreateMessageResponse>;
}

export class ProviderError extends Error {
  readonly status: number;

  constructor(message: string, status: number) {
    super(message);
    this.name = 'ProviderError';
    this.status = status;
  }
}
