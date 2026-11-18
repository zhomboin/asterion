import type { ButtonHTMLAttributes, ReactNode } from 'react';
import { formatTime, type Node } from './api';

/* ------------------------------------------------------------------ */
/* 图标：统一 24px 画布、1.6 线宽、圆角端点，详见 docs/design-system.md */
/* ------------------------------------------------------------------ */

const iconPaths = {
  overview: (
    <>
      <rect x="3.5" y="3.5" width="7" height="7" rx="2" />
      <rect x="13.5" y="3.5" width="7" height="7" rx="2" />
      <rect x="3.5" y="13.5" width="7" height="7" rx="2" />
      <rect x="13.5" y="13.5" width="7" height="7" rx="2" />
    </>
  ),
  nodes: (
    <>
      <rect x="3.5" y="4" width="17" height="6.5" rx="2" />
      <rect x="3.5" y="13.5" width="17" height="6.5" rx="2" />
      <path d="M7.5 7.25h.01M7.5 16.75h.01" />
    </>
  ),
  assets: (
    <>
      <path d="m12 3.5 8.5 4.25L12 12 3.5 7.75 12 3.5Z" />
      <path d="m3.5 12 8.5 4.25L20.5 12" />
      <path d="m3.5 16.25 8.5 4.25 8.5-4.25" />
    </>
  ),
  operations: (
    <>
      <rect x="3" y="4.5" width="18" height="15" rx="3" />
      <path d="m7.5 10 2.5 2-2.5 2M12.5 15h4" />
    </>
  ),
  alerts: (
    <>
      <path d="M6 16.5V11a6 6 0 1 1 12 0v5.5l1.5 1.5h-15L6 16.5Z" />
      <path d="M10 20.5a2 2 0 0 0 4 0" />
    </>
  ),
  settings: (
    <>
      <path d="M4 7h10M18 7h2M4 17h4M12 17h8" />
      <circle cx="16" cy="7" r="2" />
      <circle cx="10" cy="17" r="2" />
    </>
  ),
  calendar: (
    <>
      <rect x="3.5" y="5" width="17" height="15.5" rx="3" />
      <path d="M3.5 10h17M8 3v4M16 3v4" />
    </>
  ),
  pulse: <path d="M3 12h4l2.5-6 5 12 2.5-6h4" />,
  arrow: <path d="M7 17 17 7M8.5 7H17v8.5" />,
  back: <path d="m14.5 6-6 6 6 6" />,
  logout: (
    <>
      <path d="M14 4.5h3.5a2 2 0 0 1 2 2v11a2 2 0 0 1-2 2H14" />
      <path d="M10 16.5 5.5 12 10 7.5M5.5 12H15" />
    </>
  ),
  plus: <path d="M12 5v14M5 12h14" />,
  more: <path d="M6 12h.01M12 12h.01M18 12h.01" />,
} as const;

export type IconName = keyof typeof iconPaths;

export function Icon({ name, size = 18 }: { name: IconName; size?: number }) {
  return (
    <svg className="icon" width={size} height={size} viewBox="0 0 24 24" fill="none" stroke="currentColor"
      strokeWidth="1.6" strokeLinecap="round" strokeLinejoin="round" aria-hidden="true" focusable="false">
      {iconPaths[name]}
    </svg>
  );
}

/** 圆形图标按钮；tone="inverse" 为黑底白图标。 */
export function IconButton({ icon, label, tone, ...props }: {
  icon: IconName;
  label: string;
  tone?: 'inverse';
} & Omit<ButtonHTMLAttributes<HTMLButtonElement>, 'children'>) {
  return (
    <button type="button" className="icon-button" data-tone={tone} aria-label={label} title={label} {...props}>
      <Icon name={icon} />
    </button>
  );
}

/** 姓名首字母头像。 */
export function Avatar({ name, size = 'md' }: { name: string; size?: 'sm' | 'md' | 'lg' }) {
  const initials = name.trim().slice(0, 2).toUpperCase() || '?';
  return <span className="avatar" data-size={size} aria-hidden="true">{initials}</span>;
}

/** 品牌标识：黑色圆点 + 字标。 */
export function Brand() {
  return (
    <a className="wordmark" href="/overview">
      <span className="wordmark-mark" aria-hidden="true" />
      <span className="wordmark-text">Asterion</span>
    </a>
  );
}

/* ------------------------------------------------------------------ */
/* 页面结构组件                                                        */
/* ------------------------------------------------------------------ */

/** 页面标题区：可选返回链接、大标题、范围说明和右侧操作。 */
export function PageHeader({ title, scope, back, actions }: {
  title: ReactNode;
  scope?: ReactNode;
  back?: { href: string; label: string };
  actions?: ReactNode;
}) {
  return (
    <>
      {back && <a className="back-link" href={back.href}><Icon name="back" size={16} />{back.label}</a>}
      <div className="page-heading">
        <div className="page-title">
          <h1>{title}</h1>
          {scope && <p className="page-scope">{scope}</p>}
        </div>
        {actions && <div className="actions">{actions}</div>}
      </div>
    </>
  );
}

/** 卡片容器；tone="inverse" 为黑色强调卡片。 */
export function Card({ title, label, actions, tone, className, children }: {
  title?: ReactNode;
  label?: string;
  actions?: ReactNode;
  tone?: 'inverse';
  className?: string;
  children: ReactNode;
}) {
  return (
    <section className={'card' + (className ? ' ' + className : '')} data-tone={tone} aria-label={label}>
      {(title || actions) && (
        <header className="card-header">
          {title && <h2>{title}</h2>}
          {actions}
        </header>
      )}
      {children}
    </section>
  );
}

/** 分段切换控件，替代页签；保持 aria-pressed 按钮语义。 */
export function Tabs<T extends string>({ label, items, value, onChange, size }: {
  label: string;
  items: readonly (readonly [T, string])[];
  value: T;
  onChange: (value: T) => void;
  size?: 'sm';
}) {
  return (
    <nav className="tabs" data-size={size} aria-label={label}>
      {items.map(([key, text]) => (
        <button key={key} type="button" aria-pressed={key === value} onClick={() => onChange(key)}>{text}</button>
      ))}
    </nav>
  );
}

/** 主操作按钮：黑色胶囊 + 白色圆形图标。 */
export function CallToAction({ icon = 'arrow', children, ...props }: {
  icon?: IconName;
  children: ReactNode;
} & Omit<ButtonHTMLAttributes<HTMLButtonElement>, 'children'>) {
  return (
    <button type="button" className="primary cta" {...props}>
      <span className="cta-icon"><Icon name={icon} size={16} /></span>
      {children}
    </button>
  );
}

/** 进度条：value 为 0–100。 */
export function Meter({ value, label }: { value: number; label: string }) {
  const clamped = Math.max(0, Math.min(100, value));
  return (
    <span className="meter" role="meter" aria-label={label} aria-valuemin={0} aria-valuemax={100} aria-valuenow={Math.round(clamped)}>
      <span style={{ inlineSize: clamped + '%' }} />
    </span>
  );
}

/* ------------------------------------------------------------------ */
/* 数据展示组件                                                        */
/* ------------------------------------------------------------------ */

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
