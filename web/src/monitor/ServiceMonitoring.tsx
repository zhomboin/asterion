import { useState } from 'react';
import { errorMessage, isUnauthorized, request } from '../api';
import { DataTable, Pagination, Problem, Timestamp } from '../ui';
import { monitorPageSize, useMonitorPage } from './MonitorPicker';
import { useResource } from '../useResource';
import { monitorReason, type ServiceMonitor, type ServiceMonitorChange, type ServiceMonitorStatus } from './serviceMonitoring';

export const serviceMonitorRefreshMs = 5_000;

const statuses = {
  pending: { text: '等待检查', tone: 'pending' },
  healthy: { text: '正常', tone: 'online' },
  down: { text: '停机', tone: 'offline' },
  unknown: { text: '状态未知', tone: 'pending' },
  disabled: { text: '已停用' },
} as const;

function MonitorStatus({ status }: { status: ServiceMonitorStatus | 'disabled' }) {
  const display = statuses[status];
  return (
    <span className="status" data-status={'tone' in display ? display.tone : undefined}>
      <span aria-hidden="true" />{display.text}
    </span>
  );
}

function currentStatus(monitor: ServiceMonitor, unavailable: string | undefined) {
  if (!monitor.enabled) return 'disabled';
  return unavailable ? 'unknown' : monitor.status;
}

function reasonText(reason: ServiceMonitor['reason']) {
  return reason ? (monitorReason[reason] ?? '服务检查失败') : '—';
}

export function ServiceMonitorList({ monitors, unavailable, onSelect }: {
  monitors: ServiceMonitor[];
  unavailable?: string;
  onSelect: (unit: string, action: HTMLButtonElement) => void;
}) {
  const page = useMonitorPage(monitors.length);
  if (!monitors.length) return <p className="empty">暂无监控服务</p>;
  return (
    <>
    <DataTable label="服务监控列表" columns={['Unit', '状态', '最近检查', '操作']}>
      {monitors.slice(page.offset, page.offset + monitorPageSize).map((monitor) => (
        <tr key={monitor.unit}>
          <th scope="row" className="technical">{monitor.unit}</th>
          <td><MonitorStatus status={currentStatus(monitor, unavailable)} /></td>
          <td><Timestamp value={monitor.checked_at} /></td>
          <td>
            <button
              type="button"
              className="quiet"
              aria-label={'查看监控 ' + monitor.unit}
              onClick={(event) => onSelect(monitor.unit, event.currentTarget)}
            >详情</button>
          </td>
        </tr>
      ))}
    </DataTable>
    <Pagination label="纳管服务分页" offset={page.offset} pageSize={monitorPageSize} hasMore={page.hasMore} onChange={page.setOffset} />
    </>
  );
}

export function ServiceMonitorDetail({ path, monitor, ready, unavailable, onChanged, onExpired }: {
  path: string;
  monitor?: ServiceMonitor;
  ready: boolean;
  unavailable?: string;
  onChanged: () => void;
  onExpired: () => void;
}) {
  const history = useResource<{ history: ServiceMonitorChange[] }>(
    monitor ? path + '/history' : null,
    onExpired,
    monitor?.enabled ? serviceMonitorRefreshMs : 0,
  );
  const page = useMonitorPage(history.data?.history.length ?? 0);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const enabled = Boolean(monitor?.enabled);

  async function toggle() {
    setBusy(true);
    setError('');
    try {
      await request<ServiceMonitor>(path, { method: enabled ? 'DELETE' : 'PUT' });
      onChanged();
      history.refresh();
    } catch (failure) {
      if (isUnauthorized(failure)) onExpired();
      else setError(errorMessage(failure));
    } finally {
      setBusy(false);
    }
  }

  return (
    <section className="monitor-section" aria-label="当前服务监控">
      <div className="section-bar">
        <h3>持续监控</h3>
        <button className={enabled ? undefined : 'primary'} type="button" disabled={busy || !ready} onClick={() => void toggle()}>
          {busy ? '正在保存…' : enabled ? '停用监控' : '启用监控'}
        </button>
      </div>
      {error && <p className="error" role="alert">{error}</p>}
      {monitor && (
        <>
          <dl className="detail-facts">
            <div><dt>监控状态</dt><dd><MonitorStatus status={currentStatus(monitor, unavailable)} /></dd></div>
            <div><dt>最近检查</dt><dd><Timestamp value={monitor.checked_at} /></dd></div>
            <div><dt>最近变化</dt><dd><Timestamp value={monitor.changed_at} /></dd></div>
            <div><dt>最近结果</dt><dd>{monitor.enabled && unavailable ? unavailable : stateText(monitor)}</dd></div>
          </dl>
          <section className="monitor-section" aria-label="服务状态历史">
            <h3>状态历史</h3>
            {history.loading && <p role="status">正在读取状态历史…</p>}
            {history.error && <Problem error={history.error} retry={history.refresh} />}
            {history.data && !history.data.history.length && <p className="empty">暂无状态历史</p>}
            {Boolean(history.data?.history.length) && (
              <DataTable label="状态历史记录" columns={['时间', '状态', '检查结果']}>
                {history.data?.history.slice(page.offset, page.offset + monitorPageSize).map((change) => (
                  <tr key={change.id}>
                    <th scope="row"><Timestamp value={change.observed_at} /></th>
                    <td><MonitorStatus status={change.status} /></td>
                    <td>{stateText(change)}</td>
                  </tr>
                ))}
              </DataTable>
            )}
            <Pagination label="状态历史分页" offset={page.offset} pageSize={monitorPageSize} hasMore={page.hasMore} onChange={page.setOffset} />
          </section>
        </>
      )}
    </section>
  );
}

function stateText(state: Pick<ServiceMonitor, 'active' | 'sub' | 'reason'>) {
  if (!state.active) return reasonText(state.reason);
  const condition = state.active + (state.sub ? ' / ' + state.sub : '');
  return state.reason ? condition + ' · ' + reasonText(state.reason) : condition;
}
