export type ServiceVerb = 'start' | 'stop' | 'restart';
export type ProcessSignal = 'term' | 'kill';
export type AccountChange =
  | { type: 'create'; username: string }
  | { type: 'set_groups'; username: string; groups: string[] }
  | { type: 'add_key'; username: string; key: string }
  | { type: 'remove_key'; username: string; fingerprint: string };

export type OperationAction =
  | { type: 'managed'; change: ManagedChange }
  | { type: 'account'; change: AccountChange }
  | { type: 'systemd'; unit: string; verb: ServiceVerb }
  | { type: 'docker'; engine_id: string; container_id: string; verb: ServiceVerb }
  | { type: 'process'; pid: number; start_ticks: number; signal: ProcessSignal }
  | { type: 'command'; name: string; parameters: Record<string, string> };

export const managedLabels = {
  install: '安装软件', save_reload: '保存并应用配置', create: '创建容器', recreate: '重建容器',
  start: '启动容器', stop: '停止容器', restart: '重启容器',
};

export type ManagedChange = {
  key: string; verb: keyof typeof managedLabels; revision: string; digest?: string; profile?: string;
  settings?: { restart: string; ports: [number, number][]; mounts: string[] };
};

type OperationState =
  | 'prepared' | 'queued' | 'running' | 'succeeded' | 'failed' | 'timed_out'
  | 'disconnected' | 'cancelled' | 'interrupted' | 'expired';

export type CommandParameter =
  | { name: string; type: 'integer'; min: number; max: number }
  | { name: string; type: 'choice'; values: string[] }
  | { name: string; type: 'identifier'; max_length: number };

export type Command = { name: string; timeout_seconds: number; parameters: CommandParameter[] };

export type OperationCatalog = {
  managed?: string[];
  enabled: boolean;
  systemd_units: string[];
  docker_targets: { engine_id: string; container_ids: string[] }[];
  process: { term: boolean; kill: boolean };
  commands: Command[];
  accounts: { enabled: boolean; usernames: string[]; groups: string[] };
};

export type NodeOperations = {
  catalog: OperationCatalog;
  propose: (action: OperationAction, resourceName?: string, content?: string) => void;
};

export type PreparedOperation = {
  task_id: string;
  challenge: string;
  action: OperationAction;
  expires_at: number;
  step_up_required: boolean;
};

export type OperationSummary = {
  id: string;
  node_id: string;
  node_name: string;
  action: OperationAction;
  state: OperationState;
  created_at: number;
  started_at: number | null;
  finished_at: number | null;
  expires_at: number;
  actor: string;
  exit_code: number | null;
  error_code: string | null;
  cancel_requested_at: number | null;
  duration_ms: number | null;
  step_up_required: boolean;
};

export type OperationList = { operations: OperationSummary[]; has_more: boolean };
export type OperationDetail = OperationSummary & {
  can_cancel: boolean;
  output: string | null;
  truncated: boolean;
  audit: { action: string; observed_at: number; actor: string | null }[];
};

export const operationStates: Record<OperationState, string> = {
  prepared: '待确认',
  queued: '排队中',
  running: '运行中',
  succeeded: '成功',
  failed: '失败',
  timed_out: '超时',
  disconnected: '连接中断',
  cancelled: '已取消',
  interrupted: '已中断',
  expired: '已过期',
};

export const serviceVerbs = ['start', 'stop', 'restart'] as const;
export const serviceVerbLabel = { start: '启动', stop: '停止', restart: '重启' };

const namesByChange = {
  create: '创建普通账号',
  set_groups: '更新授权附加组',
  add_key: '添加 SSH 公钥',
  remove_key: '移除 SSH 公钥',
};

export function operationName(action: OperationAction): string {
  if (action.type === 'account') return namesByChange[action.change.type];
  switch (action.type) {
    case 'managed': return managedLabels[action.change.verb];
    case 'systemd': return serviceVerbLabel[action.verb] + ' systemd 服务';
    case 'docker': return serviceVerbLabel[action.verb] + '容器';
    case 'process': return action.signal === 'kill' ? '强制终止进程' : '终止进程';
    case 'command': return action.name;
  }
}

export function operationTarget(action: OperationAction): string {
  if (action.type === 'account') return action.change.username;
  switch (action.type) {
    case 'managed': return action.change.key;
    case 'systemd': return action.unit;
    case 'docker': return action.container_id;
    case 'process': return 'PID ' + action.pid;
    case 'command': return action.name;
  }
}
