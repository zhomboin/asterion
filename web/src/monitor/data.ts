export type MetricWindow = '1h' | '24h';

export type Memory = {
  total_bytes: number;
  available_bytes: number;
  used_bytes: number;
};

export type Metrics = {
  captured_at: number;
  cpu_percent: number | null;
  logical_cores: number;
  load: [number, number, number];
  memory: Memory;
  swap: Memory;
  filesystems: {
    mountpoint: string;
    fs_type: string;
    total_bytes: number;
    available_bytes: number;
  }[];
  interfaces: {
    name: string;
    rx_bytes: number;
    tx_bytes: number;
    rx_bytes_per_second: number | null;
    tx_bytes_per_second: number | null;
  }[];
};

export type MetricPoint = { observed_at: number; sample: Metrics };
export type MetricHistory = {
  samples: MetricPoint[];
  range: MetricWindow;
  interval_seconds?: number;
  bucket_seconds?: number;
  collector?: { pending: number; missed: number } | null;
  truncated?: boolean;
};

export type Process = {
  pid: number;
  start_ticks: number;
  name: string;
  state: string;
  cpu_percent: number | null;
  memory_bytes: number | null;
  started_at: number | null;
};

export type ProcessDetail = Process & {
  command_line: string[] | null;
  executable: string | null;
  parent_pid: number | null;
  user: string | null;
  runtime?: { kind: 'systemd'; unit: string } | { kind: 'docker'; container_id: string } | null;
};

export type Container = {
  id: string;
  name: string;
  image: string;
  state: string;
  status: string;
  health: string | null;
  started_at: number | null;
  restart_count: number | null;
  compose_project: string | null;
  compose_service: string | null;
  mounts: { source: string; destination: string; mode: string }[];
  ports: { ip: string | null; private_port: number; public_port: number | null; protocol: string }[];
};

export type Docker = {
  status: 'available' | 'disabled' | 'not_installed' | 'permission_denied' | 'unreachable';
  engine_id: string | null;
  engine_version: string | null;
  endpoint_fingerprint: string | null;
  containers: Container[];
  truncated?: boolean;
};

export type ContainerStats = {
  cpu_percent: number | null;
  memory_bytes: number | null;
  memory_limit_bytes: number | null;
  rx_bytes: number | null;
  tx_bytes: number | null;
};

export type ContainerLogs = { text: string; truncated: boolean };

export type Systemd = {
  status: 'available' | 'unavailable' | 'permission_denied' | 'error';
  units: Unit[];
  truncated?: boolean;
};

export type Unit = {
  name: string;
  active: string;
  sub: string;
  description: string;
  exec_start: string | null;
  working_directory: string | null;
  fragment_path: string | null;
  observed_at: number;
  source: 'systemd';
  verified: boolean;
};
