import { useState, type FormEvent } from 'react';
import type { Node, NodePage } from './api';
import { NodeDialog } from './NodeDialog';
import { Card, CallToAction, Meter, NodeStatus, PageHeader, Pagination, Problem, Timestamp, nodeStatusLabel, nodeStatuses } from './ui';
import { useResource } from './useResource';
import { OverviewAlerts } from './alerts/OverviewAlerts';

const pageSize = 50;
type Dialog = { node?: Node };
const percent = new Intl.NumberFormat('zh-CN', { maximumFractionDigits: 1 });

export function NodesPage({ manage, onExpired }: { manage: boolean; onExpired: () => void }) {
  const [filters, setFilters] = useState<Record<string, string>>({});
  const [offset, setOffset] = useState(0);
  const [dialog, setDialog] = useState<Dialog>();
  const params = new URLSearchParams({ ...filters, limit: String(pageSize), offset: String(offset) });
  const snapshot = useResource<NodePage>('/nodes?' + params, onExpired, 5_000);
  const page = snapshot.data;
  const nodes = page?.nodes ?? [];
  const counts = { pending: 0, online: 0, offline: 0 };
  for (const node of nodes) counts[node.status]++;
  const partial = Boolean(offset || page?.has_more);

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
            <NodeRow key={node.id} node={node} manage={manage} onManage={() => setDialog({ node })} />
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
        scope={manage ? '登记、筛选与管理受监控节点' : '节点健康、进行中告警与最近心跳'}
        actions={<CallToAction icon="plus" onClick={() => setDialog({})}>登记节点</CallToAction>}
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
          {page && <HealthCard counts={counts} total={nodes.length} partial={partial} />}
          <OverviewAlerts onExpired={onExpired} />
          <Card className="overview-nodes" title="节点" actions={<a className="text-link" href="/nodes">管理节点</a>}>
            {list}
          </Card>
        </div>
      )}
      {dialog && (
        <NodeDialog
          node={dialog.node}
          onClose={() => setDialog(undefined)}
          onUpdated={snapshot.refresh}
          onExpired={onExpired}
        />
      )}
    </>
  );
}

function HealthCard({ counts, total, partial }: { counts: Record<Node['status'], number>; total: number; partial: boolean }) {
  const ratio = (count: number) => (total ? (count / total) * 100 : 0);
  return (
    <Card className="health-card" title={partial ? '本页节点健康' : '节点健康'}>
      <p className="figure">{percent.format(ratio(counts.online))}<small>%</small></p>
      <p className="muted">在线率 · 共 {total} 个节点</p>
      <dl className="legend" aria-label={partial ? '本页节点状态' : '节点状态'}>
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

function NodeRow({ node, manage, onManage }: { node: Node; manage: boolean; onManage: () => void }) {
  const environment = node.kind === 'wsl' ? ['WSL', node.wsl_distro].filter(Boolean).join(' · ') : 'Linux';

  return (
    <li className="node-row">
      <div className="node-identity">
        <h2><a className="text-link" href={'/nodes/' + encodeURIComponent(node.id)}>{node.name}</a></h2>
        <p>{environment}</p>
        {node.tags.length > 0 && <ul className="tags" aria-label="标签">{node.tags.map((tag, index) => <li key={index}>{tag}</li>)}</ul>}
        {manage && node.notes && <p className="notes">{node.notes}</p>}
      </div>
      <dl className="node-meta">
        <div><dt>状态</dt><dd><NodeStatus node={node} /></dd></div>
        <div><dt>最近心跳</dt><dd><Timestamp value={node.last_seen_at} /></dd></div>
        <div><dt>系统</dt><dd>{[node.os, node.arch].filter(Boolean).join(' · ') || '—'}</dd></div>
        <div><dt>Agent 版本</dt><dd>{node.agent_version ?? '—'}</dd></div>
      </dl>
      {manage && <button className="quiet" type="button" onClick={onManage} aria-label={'管理 ' + node.name}>管理</button>}
    </li>
  );
}
