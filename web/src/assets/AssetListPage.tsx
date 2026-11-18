import { useState, type FormEvent } from 'react';
import type { NodePage } from '../api';
import { CallToAction, DataTable, PageHeader, Pagination, Problem, Timestamp } from '../ui';
import { useResource } from '../useResource';
import { AssetForm } from './AssetForm';
import { managementLabels, managementOptions, type AssetResults } from './data';

const pageSize = 12;

export function AssetListPage({ onExpired }: { onExpired: () => void }) {
  const initialNode = new URLSearchParams(window.location.search).get('node_id') ?? '';
  const [filters, setFilters] = useState<Record<string, string>>(initialNode ? { node_id: initialNode } : {});
  const [offset, setOffset] = useState(0);
  const [creating, setCreating] = useState(false);
  const params = new URLSearchParams({ ...filters, offset: String(offset), limit: String(pageSize) });
  const list = useResource<AssetResults>('/assets?' + params, onExpired);
  const nodes = useResource<NodePage>('/nodes?limit=100', onExpired);

  function search(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const fields = new FormData(event.currentTarget);
    const next: Record<string, string> = {};
    for (const name of ['q', 'node_id', 'management', 'tag']) {
      const value = String(fields.get(name) ?? '').trim();
      if (value) next[name] = value;
    }
    setFilters(next);
    setOffset(0);
  }

  return (
    <>
      <PageHeader
        title="服务资产"
        scope="人工登记的服务及其运行实例"
        actions={<CallToAction icon="plus" onClick={() => setCreating(true)} disabled={creating}>登记资产</CallToAction>}
      />
      {creating && (
        <AssetForm
          onExpired={onExpired}
          onCancel={() => setCreating(false)}
          onSaved={(asset) => window.location.assign('/assets/' + encodeURIComponent(asset.id))}
        />
      )}
      <form className="filter-bar" onSubmit={search}>
        <label>搜索<input name="q" type="search" placeholder="名称、说明或路径" maxLength={100} /></label>
        <label>节点<select name="node_id" defaultValue={initialNode}>
          <option value="">全部节点</option>
          {initialNode && !nodes.data?.nodes.some((node) => node.id === initialNode) && <option value={initialNode}>{initialNode}</option>}
          {nodes.data?.nodes.map((node) => <option key={node.id} value={node.id}>{node.name}</option>)}
        </select></label>
        <label>管理方式<select name="management">
          <option value="">全部方式</option>
          {managementOptions.map(([value, label]) => <option key={value} value={value}>{label}</option>)}
        </select></label>
        <label>标签<input name="tag" maxLength={32} /></label>
        <button type="submit">筛选</button>
      </form>
      {nodes.error && <Problem error={nodes.error} retry={nodes.refresh} />}
      {list.loading && <p role="status">正在读取资产…</p>}
      {list.error && <Problem error={list.error} retry={list.refresh} />}
      {list.data && list.data.assets.length === 0 && <p className="empty">没有匹配的服务资产</p>}
      {Boolean(list.data?.assets.length) && (
        <DataTable label="服务资产目录" columns={['资产', '节点', '管理方式', '来源', '最后观测', '编辑时间']}>
          {list.data?.assets.map((asset) => (
            <tr key={asset.id}>
              <th scope="row">
                <a className="text-link" href={'/assets/' + encodeURIComponent(asset.id)}>{asset.name}</a>
                {asset.tags.length > 0 && <ul className="tags" aria-label="标签">{asset.tags.map((tag, index) => <li key={index}>{tag}</li>)}</ul>}
              </th>
              <td><a className="text-link" href={'/nodes/' + encodeURIComponent(asset.node_id)}>{asset.node_name}</a></td>
              <td>{managementLabels[asset.management]}</td>
              <td>人工登记 · {asset.verified_at == null ? '未核实' : '已核实'}</td>
              <td><Timestamp value={asset.last_seen_at} /></td>
              <td><Timestamp value={asset.updated_at} /></td>
            </tr>
          ))}
        </DataTable>
      )}
      <Pagination label="资产分页" offset={offset} pageSize={pageSize} hasMore={list.data?.has_more} onChange={setOffset} />
    </>
  );
}
