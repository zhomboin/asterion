export const managementLabels = {
  docker: 'Docker',
  compose: 'Docker Compose',
  systemd: 'systemd',
  process: '进程',
  other: '其他',
};

export type Management = keyof typeof managementLabels;

export type AssetDraft = {
  node_id: string;
  name: string;
  management: Management;
  description: string;
  repo_url: string | null;
  deploy_path: string | null;
  config_path: string | null;
  tags: string[];
  notes: string;
  verified: boolean;
};

export type Asset = Omit<AssetDraft, 'verified'> & {
  id: string;
  node_name: string;
  verified_at: number | null;
  created_at: number;
  updated_at: number;
  last_seen_at: number | null;
  source: 'manual';
};

export type AssetDetail = Asset & { bindings: Binding[] };
export type AssetResults = { assets: Asset[]; has_more: boolean };

export type Discovery = {
  id: string;
  kind: 'docker' | 'systemd' | 'process';
  resource_id: string;
  name: string;
  observed_at: number;
  source: 'agent';
};

export type Binding = Discovery & { bound_at: number };
export type Discoveries = { discoveries: Discovery[]; has_more: boolean };

export const managementOptions = Object.entries(managementLabels);
