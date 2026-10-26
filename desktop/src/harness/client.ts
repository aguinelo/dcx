export interface HarnessProject {
  path: string;
}

export interface HarnessSession {
  session_id: string;
  workspace: string;
  provider: string;
  model?: string;
  message_count: number;
  created_at_unix_ms: number;
  updated_at_unix_ms: number;
}

export interface HarnessMessage {
  role: 'user' | 'assistant';
  content: string;
  created_at_unix_ms: number;
  cost_usd?: number;
  duration_ms?: number;
  input_tokens?: number;
  output_tokens?: number;
  cache_creation_input_tokens?: number;
  cache_read_input_tokens?: number;
}

export interface HarnessSessionDetail extends HarnessSession {
  messages: HarnessMessage[];
}

export interface HarnessRun {
  schema_version: number;
  run_id: string;
  session_id: string;
  workspace: string;
  request: string;
  status: string;
  created_at_unix_ms: number;
}

export interface HarnessEvent {
  event: string;
  data: Record<string, unknown>;
  created_at_unix_ms?: number;
}

export interface HarnessConfig {
  provider: string;
  model?: string;
  web_enabled: boolean;
  max_tool_calls: number;
}

export interface HarnessAccount {
  name: string;
  email?: string;
}

export interface HarnessClientOptions {
  baseUrl?: string;
  fetchImpl?: typeof fetch;
}

/** Narrow HTTP/SSE client for the local Rust Harness daemon. */
export class HarnessClient {
  private readonly baseUrl: string;
  private readonly fetchImpl: typeof fetch;

  constructor(options: HarnessClientOptions = {}) {
    this.baseUrl = (options.baseUrl ?? 'http://127.0.0.1:8787').replace(/\/$/, '');
    this.fetchImpl = options.fetchImpl ?? fetch;
  }

  async health(): Promise<{ status: string }> {
    return this.get('/health');
  }

  async config(workspace: string): Promise<HarnessConfig> {
    return this.get(`/config?workspace=${encodeURIComponent(workspace)}`);
  }

  async setConfig(
    workspace: string,
    updates: Partial<Pick<HarnessConfig, 'provider' | 'web_enabled' | 'max_tool_calls'>>,
  ): Promise<HarnessConfig> {
    return this.post('/config', { workspace, ...updates });
  }

  async account(workspace: string): Promise<HarnessAccount> {
    return this.get(`/account?workspace=${encodeURIComponent(workspace)}`);
  }

  async projects(root: string): Promise<HarnessProject[]> {
    const response = await this.get<{ projects: string[] }>(
      `/projects?root=${encodeURIComponent(root)}`,
    );
    return response.projects.map((path) => ({ path }));
  }

  async sessions(workspace: string): Promise<HarnessSession[]> {
    const response = await this.get<{ sessions: HarnessSession[] }>(
      `/sessions?workspace=${encodeURIComponent(workspace)}`,
    );
    return response.sessions;
  }

  async session(workspace: string, sessionId: string): Promise<HarnessSessionDetail> {
    return this.get(
      `/session?workspace=${encodeURIComponent(workspace)}&session_id=${encodeURIComponent(sessionId)}`,
    );
  }

  async promote(workspace: string, sessionId: string): Promise<{ status: string; request: string }> {
    return this.post('/runs/from-session', { workspace, session_id: sessionId });
  }

  async approve(workspace: string, sessionId: string, approved = true): Promise<unknown> {
    return this.post('/runs/from-session/approve', {
      workspace,
      session_id: sessionId,
      approved,
    });
  }

  async start(workspace: string, sessionId: string): Promise<{ run_id: string; status: string }> {
    return this.post('/runs/from-session/start', { workspace, session_id: sessionId });
  }

  async status(workspace: string, runId: string): Promise<HarnessRun> {
    return this.get(
      `/runs/status?workspace=${encodeURIComponent(workspace)}&run_id=${encodeURIComponent(runId)}`,
    );
  }

  async sendInput(runId: string, input: string): Promise<void> {
    await this.post('/runs/input', { run_id: runId, input });
  }

  async cancel(workspace: string, runId: string): Promise<void> {
    await this.post('/runs/cancel', { workspace, run_id: runId });
  }

  async events(
    workspace: string,
    runId: string,
    onEvent: (event: HarnessEvent) => void,
    signal?: AbortSignal,
  ): Promise<void> {
    const response = await this.fetchImpl(
      `${this.baseUrl}/runs/events?workspace=${encodeURIComponent(workspace)}&run_id=${encodeURIComponent(runId)}`,
      signal ? { signal } : {},
    );
    if (!response.ok || !response.body) {
      throw new Error(`Harness events HTTP ${response.status}`);
    }
    const reader = response.body.getReader();
    const decoder = new TextDecoder();
    let buffer = '';
    let event = 'message';
    for (;;) {
      const chunk = await reader.read();
      if (chunk.done) break;
      buffer += decoder.decode(chunk.value, { stream: true });
      const records = buffer.split('\n\n');
      buffer = records.pop() ?? '';
      for (const record of records) {
        const eventLine = record.split('\n').find((line) => line.startsWith('event: '));
        const dataLine = record.split('\n').find((line) => line.startsWith('data: '));
        if (eventLine) event = eventLine.slice('event: '.length);
        if (dataLine) {
          onEvent({ event, data: JSON.parse(dataLine.slice('data: '.length)) });
        }
      }
    }
  }

  async chat(
    workspace: string,
    message: string,
    sessionId: string | undefined,
    onEvent: (event: HarnessEvent) => void,
    signal?: AbortSignal,
  ): Promise<string | undefined> {
    const response = await this.fetchImpl(`${this.baseUrl}/chat`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify({ workspace, message, ...(sessionId ? { session_id: sessionId } : {}) }),
      ...(signal ? { signal } : {}),
    });
    if (!response.ok || !response.body) {
      throw new Error(`Harness chat HTTP ${response.status}`);
    }
    const reader = response.body.getReader();
    const decoder = new TextDecoder();
    let buffer = '';
    let session: string | undefined;
    for (;;) {
      const chunk = await reader.read();
      if (chunk.done) break;
      buffer += decoder.decode(chunk.value, { stream: true });
      const records = buffer.split('\n\n');
      buffer = records.pop() ?? '';
      for (const record of records) {
        const eventLine = record.split('\n').find((line) => line.startsWith('event: '));
        const dataLine = record.split('\n').find((line) => line.startsWith('data: '));
        if (!dataLine) continue;
        const event = eventLine?.slice('event: '.length) ?? 'message';
        const data = JSON.parse(dataLine.slice('data: '.length)) as Record<string, unknown>;
        if (event === 'session' && typeof data.session_id === 'string') session = data.session_id;
        onEvent({ event, data });
      }
    }
    return session;
  }

  private async get<T>(path: string): Promise<T> {
    const response = await this.fetchImpl(`${this.baseUrl}${path}`);
    return this.read<T>(response);
  }

  private async post<T>(path: string, body: unknown): Promise<T> {
    const response = await this.fetchImpl(`${this.baseUrl}${path}`, {
      method: 'POST',
      headers: { 'content-type': 'application/json' },
      body: JSON.stringify(body),
    });
    return this.read<T>(response);
  }

  private async read<T>(response: Response): Promise<T> {
    const text = await response.text();
    let payload: unknown;
    try {
      payload = text ? JSON.parse(text) : undefined;
    } catch {
      throw new Error(`Harness retornou JSON inválido (HTTP ${response.status})`);
    }
    if (!response.ok) {
      const message =
        typeof payload === 'object' && payload && 'error' in payload
          ? String((payload as { error: unknown }).error)
          : `HTTP ${response.status}`;
      throw new Error(message);
    }
    return payload as T;
  }
}
