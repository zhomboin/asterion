export type ServiceMonitorStatus = 'pending' | 'healthy' | 'down' | 'unknown';

type ServiceMonitorReason = 'offline' | 'busy' | 'timeout' | 'unsupported' | 'not_found'
  | 'permission_denied' | 'unavailable' | 'query_failed' | 'invalid_response' | 'transition';

type ServiceState = {
  active: string | null;
  sub: string | null;
  reason: ServiceMonitorReason | null;
};

export type ServiceMonitor = ServiceState & {
  unit: string;
  enabled: boolean;
  status: ServiceMonitorStatus;
  checked_at: number | null;
  changed_at: number | null;
};

export type ServiceMonitorChange = ServiceState & {
  id: number;
  status: Exclude<ServiceMonitorStatus, 'pending'>;
  observed_at: number;
};

export const monitorReason: Record<ServiceMonitorReason, string> = {
  offline: 'SSH 离线',
  busy: 'SSH 正忙',
  timeout: '检查超时',
  unsupported: 'SSH 不支持服务检查',
  not_found: '未找到服务',
  permission_denied: '没有读取权限',
  unavailable: 'systemd 不可用',
  query_failed: '服务检查失败',
  invalid_response: '服务状态返回无效',
  transition: '服务状态正在变化',
};
