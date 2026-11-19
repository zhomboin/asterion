import { useState, type FormEvent } from 'react';
import type { Node, NodePage, NodeStatusCounts } from './api';
import { Card, CallToAction, Meter, NodeStatus, PageHeader, Pagination, Problem, Timestamp, fraction, nodeStatusLabel, nodeStatuses } from './ui';
import { useResource } from './useResource';
import { OverviewAlerts } from './alerts/OverviewAlerts';

const pageSize = 12;
const percent = new Intl.NumberFormat('zh-CN', { maximumFractionDigits: 1 });

export function NodesPage({ manage, onExpired }: { manage: boolean; onExpired: () => void }) {
  const [filters, setFilters] = useState<Record<string, string>>({});
  const [offset, setOffset] = useState(0);
  const params = new URLSearchParams({ ...filters, limit: String(pageSize), offset: String(offset) });
  if (!manage) params.set('metrics', 'true');
  const snapshot = useResource<NodePage>('/nodes?' + params, onExpired, 5_000);
  const page = snapshot.data;
  const nodes = page?.nodes ?? [];

  function search(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const fields = new FormData(event.currentTarget);
    const next: Record<string, string> = {};
    for (const name of ['q', 'tag', 'status']) {
      const value = String(fields.get(name) ?? '').trim();
      if (value) next[name] = value;
    }
    setFilters(next);
    setOffset(0);
  }

  const list = (
    <>
      {page && !nodes.length && <p className="empty">{Object.keys(filters).length ? '没有匹配的节点' : '暂无节点'}</p>}
      {page && nodes.length > 0 && (
        <ul className="node-list" aria-label="节点列表">
          {nodes.map((node) => (
            <NodeRow key={node.id} node={node} manage={manage} />
          ))}
        </ul>
      )}
      <Pagination label="节点分页" offset={offset} pageSize={pageSize} hasMore={page?.has_more} onChange={setOffset} />
    </>
  );

  return (
    <>
      <PageHeader
        title={manage ? '节点' : '总览'}
        scope={manage ? '登记、筛选与管理受监控节点' : '节点健康、资源用量与进行中告警'}
        actions={<CallToAction icon="plus" onClick={() => window.location.assign('/nodes/new')}>接入服务器</CallToAction>}
      />
      {manage && (
        <form className="filter-bar" onSubmit={search}>
          <label>搜索<input type="search" name="q" placeholder="名称或 ID" maxLength={100} /></label>
          <label>标签<input name="tag" maxLength={32} /></label>
          <label>状态<select name="status">
            <option value="">全部状态</option>
            {nodeStatuses.map((status) => <option value={status} key={status}>{nodeStatusLabel[status]}</option>)}
          </select></label>
          <button type="submit">筛选</button>
        </form>
      )}
      {snapshot.error && <Problem error={page ? '节点状态未更新。' : snapshot.error} retry={snapshot.refresh} />}
      {snapshot.loading && <p role="status" className="muted">正在加载节点…</p>}
      {manage ? list : (
        <div className="overview-grid">
          {page?.status_counts && <HealthCard counts={page.status_counts} />}
          <OverviewAlerts onExpired={onExpired} />
          <Card className="overview-nodes" title="节点" actions={<a className="text-link" href="/nodes">管理节点</a>}>
            {list}
          </Card>
        </div>
      )}
    </>
  );
}

function HealthCard({ counts }: { counts: NodeStatusCounts }) {
  const total = counts.pending + counts.online + counts.offline;
  const ratio = (count: number) => (total ? (count / total) * 100 : 0);
  return (
    <Card className="health-card" title="节点健康">
      <p className="figure">{percent.format(ratio(counts.online))}<small>%</small></p>
      <p className="muted">在线率 · 共 {total} 个节点</p>
      <dl className="legend" aria-label="节点状态">
        {nodeStatuses.map((status) => (
          <div key={status} data-status={status}>
            <dt><span className="dot" aria-hidden="true" />{nodeStatusLabel[status]}</dt>
            <dd>
              <Meter value={ratio(counts[status])} label={nodeStatusLabel[status] + '占比'} />
              <span className="legend-value">{counts[status]}</span>
            </dd>
          </div>
        ))}
      </dl>
    </Card>
  );
}

function NodeRow({ node, manage }: { node: Node; manage: boolean }) {
  const environment = node.kind === 'wsl' ? ['WSL', node.wsl_distro].filter(Boolean).join(' · ') : 'Linux';

  return (
    <li className="node-row">
      <div className="node-identity">
        <h2><a className="text-link" href={'/nodes/' + encodeURIComponent(node.id)}>{node.name}</a></h2>
        <p>{environment}</p>
        {node.host_name && <p>宿主机：{node.host_name}</p>}
        {node.tags.length > 0 && <ul className="tags" aria-label="标签">{node.tags.map((tag, index) => <li key={index}>{tag}</li>)}</ul>}
        {manage && node.notes && <p className="notes">{node.notes}</p>}
      </div>
      <dl className="node-meta">
        <div><dt>状态</dt><dd><NodeStatus node={node} /></dd></div>
        <div><dt>最近心跳</dt><dd><Timestamp value={node.last_seen_at} /></dd></div>
        <div><dt>系统</dt><dd>{[node.os, node.arch].filter(Boolean).join(' · ') || '—'}</dd></div>
        <div><dt>Agent 版本</dt><dd>{node.agent_version ?? '—'}</dd></div>
        {!manage && (
          <>
            <div><dt>CPU</dt><dd>{fraction(node.metrics?.cpu_percent ?? null)}</dd></div>
            <div><dt>内存</dt><dd>{fraction(node.metrics?.memory_percent ?? null)}</dd></div>
            <div><dt>磁盘</dt><dd>{fraction(node.metrics?.disk_percent ?? null)}</dd></div>
            <div><dt>最近采样</dt><dd><Timestamp value={node.metrics?.observed_at ?? null} /></dd></div>
          </>
        )}
      </dl>
      {manage && <div className="row-actions">
        <a className="text-link" href={'/nodes/' + node.id + '/deploy'} aria-label={'Agent 接入 ' + node.name}>Agent 接入</a>
        <a className="text-link" href={'/nodes/' + node.id + '/manage'} aria-label={'管理 ' + node.name}>管理</a>
      </div>}
    </li>
  );
}
