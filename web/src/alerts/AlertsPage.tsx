import { useState, type FormEvent } from 'react';
import type { NodePage } from '../api';
import { DataTable, fraction, Pagination, Problem, Timestamp } from '../ui';
import { useResource } from '../useResource';
import { AlertRulesPanel } from './AlertRulesPanel';
import { alertMetric, type Alerts } from './data';

const pageSize = 50;

export function AlertsPage({ onExpired }: { onExpired: () => void }) {
  const [view, setView] = useState<'events' | 'rules'>('events');
  const nodes = useResource<NodePage>('/nodes?limit=100', onExpired);
  return (
    <>
      <div className="page-heading"><h1>告警</h1></div>
      <nav className="tabs" aria-label="告警视图">
        <button type="button" aria-pressed={view === 'events'} onClick={() => setView('events')}>事件</button>
        <button type="button" aria-pressed={view === 'rules'} onClick={() => setView('rules')}>规则</button>
      </nav>
      {nodes.error && <Problem error={nodes.error} retry={nodes.refresh} />}
      {view === 'events' && <AlertEvents nodes={nodes.data?.nodes ?? []} onExpired={onExpired} />}
      {view === 'rules' && <AlertRulesPanel nodes={nodes.data?.nodes ?? []} onExpired={onExpired} />}
    </>
  );
}

function AlertEvents({ nodes, onExpired }: { nodes: NodePage['nodes']; onExpired: () => void }) {
  const [filters, setFilters] = useState({ state: 'active', node_id: '' });
  const [offset, setOffset] = useState(0);
  const params = new URLSearchParams({ state: filters.state, limit: String(pageSize), offset: String(offset) });
  if (filters.node_id) params.set('node_id', filters.node_id);
  const alerts = useResource<Alerts>('/alerts?' + params, onExpired, 10_000);

  function search(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const values = new FormData(event.currentTarget);
    setFilters({ state: String(values.get('state') ?? 'active'), node_id: String(values.get('node_id') ?? '') });
    setOffset(0);
  }

  return (
    <section aria-label="告警事件">
      <form className="filter-bar" onSubmit={search}>
        <label>状态<select name="state" defaultValue={filters.state}>
          <option value="active">进行中</option><option value="resolved">已恢复</option><option value="all">全部状态</option>
        </select></label>
        <label>节点<select name="node_id" defaultValue={filters.node_id}>
          <option value="">全部节点</option>
          {nodes.map((node) => <option key={node.id} value={node.id}>{node.name}</option>)}
        </select></label>
        <button type="submit">筛选</button>
      </form>
      {alerts.loading && <p role="status">正在读取告警…</p>}
      {alerts.error && <Problem error={alerts.error} retry={alerts.refresh} />}
      {alerts.data && alerts.data.alerts.length === 0 && <p className="empty">暂无匹配告警</p>}
      {Boolean(alerts.data?.alerts.length) && (
        <DataTable label="告警事件" columns={['指标', '节点', '状态', '观测值 / 阈值', '触发时间', '恢复时间']}>
          {alerts.data?.alerts.map((alert) => (
            <tr key={alert.id}>
              <th scope="row">{alertMetric[alert.metric]}</th>
              <td><a className="text-link" href={'/nodes/' + encodeURIComponent(alert.node_id)}>{alert.node_name}</a></td>
              <td className="operation-state" data-state={alert.status === 'active' ? 'failed' : 'succeeded'}>{alert.status === 'active' ? '进行中' : '已恢复'}</td>
              <td>{alert.metric === 'offline' ? '—' : fraction(alert.value) + ' / ' + fraction(alert.threshold)}</td>
              <td><Timestamp value={alert.opened_at} /></td><td><Timestamp value={alert.resolved_at} /></td>
            </tr>
          ))}
        </DataTable>
      )}
      <Pagination label="告警分页" offset={offset} pageSize={pageSize} hasMore={alerts.data?.has_more} onChange={setOffset} />
    </section>
  );
}
