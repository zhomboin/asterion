export const alertMetric = {
  offline: '节点离线',
  cpu: 'CPU 利用率',
  memory: '内存利用率',
  disk: '文件系统利用率',
};

export type AlertMetric = keyof typeof alertMetric;

export type RuleDraft = {
  node_id: string;
  metric: AlertMetric;
  threshold: number | null;
  for_seconds: number;
  enabled: boolean;
};

export type AlertRule = RuleDraft & {
  id: string;
  node_name: string;
  created_at: number;
  updated_at: number;
};

export type Alert = {
  id: string;
  rule_id: string;
  node_id: string;
  node_name: string;
  metric: AlertMetric;
  status: 'active' | 'resolved';
  value: number | null;
  threshold: number | null;
  opened_at: number;
  updated_at: number;
  resolved_at: number | null;
};

export type Alerts = { alerts: Alert[]; has_more: boolean };
export type Rules = { rules: AlertRule[]; has_more: boolean };
