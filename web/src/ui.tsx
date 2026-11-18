import type { ReactNode } from 'react';
import { formatTime, type Node } from './api';

export const nodeStatuses = ['online', 'offline', 'pending'] as const;
export const nodeStatusLabel = { pending: '待连接', online: '在线', offline: '离线' };

export function NodeStatus({ node }: { node: Node }) {
  return (
    <span className="status" data-status={node.status}>
      <span aria-hidden="true" />
      {node.kind === 'wsl' && node.status === 'offline' ? 'Agent 失联' : nodeStatusLabel[node.status]}
    </span>
  );
}

export function Timestamp({ value }: { value: number | null }) {
  if (value == null) return <span className="muted">—</span>;
  const date = new Date(value * 1_000);
  if (!Number.isFinite(date.getTime())) return <span className="muted">—</span>;
  return <time dateTime={date.toISOString()}>{formatTime(value)}</time>;
}

export function Problem({ error, retry }: { error: string; retry: () => void }) {
  return (
    <div className="notice" role="alert">
      <p>{error}</p>
      <button type="button" onClick={retry}>重试</button>
    </div>
  );
}

export function DataTable({ label, columns, children }: {
  label: string;
  columns: ReactNode[];
  children: ReactNode;
}) {
  return (
    <div className="table-scroll" tabIndex={0} role="region" aria-label={label}>
      <table className="data-table" aria-label={label}>
        <thead><tr>{columns.map((column, index) => <th key={index} scope="col">{column}</th>)}</tr></thead>
        <tbody>{children}</tbody>
      </table>
    </div>
  );
}

export function Pagination({ label, offset, pageSize, hasMore, onChange, maxOffset = 100_000, disabled = false }: {
  label: string;
  offset: number;
  pageSize: number;
  hasMore?: boolean;
  onChange: (offset: number) => void;
  maxOffset?: number;
  disabled?: boolean;
}) {
  if (offset === 0 && !hasMore) return null;
  return (
    <nav className="pagination" aria-label={label}>
      <button type="button" disabled={disabled || offset === 0} onClick={() => onChange(offset - pageSize)}>上一页</button>
      <span>第 {offset / pageSize + 1} 页</span>
      <button type="button" disabled={disabled || !hasMore || offset + pageSize > maxOffset} onClick={() => onChange(offset + pageSize)}>下一页</button>
    </nav>
  );
}

const byteUnits = ['B', 'KiB', 'MiB', 'GiB', 'TiB', 'PiB'];
const measurements = new Intl.NumberFormat('zh-CN', { maximumFractionDigits: 1 });

export function bytes(value: number | null) {
  if (value == null) return '—';
  let magnitude = value;
  let unit = 0;
  while (magnitude >= 1_024 && unit < byteUnits.length - 1) {
    magnitude /= 1_024;
    unit += 1;
  }
  return measurements.format(magnitude) + ' ' + byteUnits[unit];
}

export function fraction(value: number | null) {
  return value == null ? '—' : measurements.format(value) + '%';
}

export function perSecond(value: number | null) {
  return value == null ? '—' : bytes(value) + '/s';
}
