export interface Session {
  username: string;
}

export interface NodeDraft {
  name: string;
  kind: 'linux' | 'wsl';
  notes: string;
  host_name?: string | null;
}

export interface NodeMetrics {
  observed_at: number;
  cpu_percent: number | null;
  memory_percent: number | null;
  disk_percent: number | null;
}

export interface Node extends NodeDraft {
  id: string;
  os: string | null;
  arch: string | null;
  wsl_distro: string | null;
  ssh_profile: string | null;
  ssh_auth?: { method: 'key' | 'password'; identity_file?: string };
  ssh_error: string | null;
  last_seen_at: number | null;
  status: 'pending' | 'online' | 'offline';
  metrics?: NodeMetrics;
}

export type NodeStatusCounts = Record<Node['status'], number>;

export interface NodePage {
  nodes: Node[];
  has_more: boolean;
  status_counts?: NodeStatusCounts;
}

export class ApiError extends Error {
  readonly status: number;

  constructor(status: number, message: string) {
    super(message);
    this.name = 'ApiError';
    this.status = status;
  }
}

type RequestOptions = {
  method?: 'GET' | 'POST' | 'PUT' | 'PATCH' | 'DELETE';
  body?: unknown;
  signal?: AbortSignal;
  keepalive?: boolean;
  timeoutMs?: number;
};

export function request<T>(path: string, options?: RequestOptions & { response?: 'json' }): Promise<T>;
export function request(path: string, options: RequestOptions & { response: 'empty' }): Promise<void>;
export async function request<T>(
  path: string,
  options: RequestOptions & { response?: 'json' | 'empty' } = {},
): Promise<T | void> {
  const { method = 'GET', body, signal, keepalive } = options;
  const timeout = AbortSignal.timeout(options?.timeoutMs ?? 12_000);
  const headers = new Headers({ Accept: 'application/json' });
  const localSession = globalThis.location?.protocol === 'http:' ? globalThis.sessionStorage : undefined;
  const sessionHeader = 'X-Asterion-Local-Session';
  const token = localSession?.getItem(sessionHeader);
  if (token) headers.set(sessionHeader, token);
  if (method !== 'GET') headers.set('X-Asterion-CSRF', '1');
  if (body !== undefined) headers.set('Content-Type', 'application/json');

  const response = await fetch('/api/v1' + path, {
    method,
    headers,
    body: body === undefined ? undefined : JSON.stringify(body),
    credentials: 'same-origin',
    cache: 'no-store',
    redirect: 'error',
    keepalive,
    signal: signal ? AbortSignal.any([signal, timeout]) : timeout,
  });

  if (!response.ok) {
    const error: unknown = await response.json().catch(() => null);
    const message = typeof error === 'object' && error !== null
      && 'message' in error && typeof error.message === 'string'
      ? error.message
      : '中心未能处理请求';
    throw new ApiError(response.status, message);
  }

  if (localSession && path === '/auth/login') {
    const issued = response.headers.get(sessionHeader);
    if (issued) localSession.setItem(sessionHeader, issued);
  }
  if (localSession && path === '/auth/logout') localSession.removeItem(sessionHeader);
  if (response.status === 204 || options.response === 'empty') return;
  return response.json() as Promise<T>;
}

export function isUnauthorized(error: unknown): boolean {
  return error instanceof ApiError && error.status === 401;
}

export function errorMessage(error: unknown): string {
  return error instanceof ApiError ? error.message : '无法连接中心，请重试。';
}

const dateFormat = new Intl.DateTimeFormat('zh-CN', { dateStyle: 'short', timeStyle: 'medium' });

export function formatTime(seconds: number): string {
  return dateFormat.format(seconds * 1_000);
}
