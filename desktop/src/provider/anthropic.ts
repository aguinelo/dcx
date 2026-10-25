import {
  type ContentBlock,
  type CreateMessageRequest,
  type CreateMessageResponse,
  type Provider,
  ProviderError,
} from './types.ts';

const DEFAULT_BASE_URL = 'https://api.anthropic.com';
const API_VERSION = '2023-06-01';

export interface AnthropicProviderOptions {
  apiKey: string;
  baseUrl?: string;
  /** Extra beta headers, comma-joined into `anthropic-beta`. */
  betas?: string[];
}

/**
 * Anthropic Messages API over plain `fetch`.
 *
 * No SDK on purpose: the request body is six fields, and a dependency here would
 * be version-coupled to the agent loop for no gain.
 */
export function createAnthropicProvider(options: AnthropicProviderOptions): Provider {
  const baseUrl = options.baseUrl ?? DEFAULT_BASE_URL;

  return {
    name: 'anthropic',

    async createMessage(request: CreateMessageRequest): Promise<CreateMessageResponse> {
      const betaHeader = options.betas?.length ? options.betas.join(',') : undefined;

      const headers: Record<string, string> = {
        'content-type': 'application/json',
        'anthropic-version': API_VERSION,
        'x-api-key': options.apiKey,
      };
      if (betaHeader) headers['anthropic-beta'] = betaHeader;

      const response = await fetch(`${baseUrl}/v1/messages`, {
        method: 'POST',
        headers,
        ...(request.signal ? { signal: request.signal } : {}),
        body: JSON.stringify({
          model: request.model,
          max_tokens: request.maxTokens,
          system: request.system,
          messages: request.messages,
          tools: request.tools.map((tool) => ({
            name: tool.name,
            description: tool.description,
            input_schema: tool.input_schema,
          })),
        }),
      });

      if (!response.ok) {
        const body = await response.text();
        throw new ProviderError(
          `provider request failed: ${response.status} ${body.slice(0, 500)}`,
          response.status,
        );
      }

      const payload = (await response.json()) as {
        content: ContentBlock[];
        stop_reason: CreateMessageResponse['stopReason'];
        usage?: { input_tokens?: number; output_tokens?: number };
      };

      return {
        content: payload.content,
        stopReason: payload.stop_reason ?? 'end_turn',
        usage: {
          inputTokens: payload.usage?.input_tokens ?? 0,
          outputTokens: payload.usage?.output_tokens ?? 0,
        },
      };
    },
  };
}
