import { useState, type FormEvent } from 'react';
import { errorMessage, isUnauthorized, request } from '../api';
import { DataTable, Pagination, Problem, Timestamp } from '../ui';
import { useResource } from '../useResource';
import { managementLabels, type AssetDetail, type Discoveries } from './data';

const pageSize = 12;

export function AssetBindings({ asset, onChanged, onExpired }: {
  asset: AssetDetail;
  onChanged: () => void;
  onExpired: () => void;
}) {
  const [adding, setAdding] = useState(false);
  const [offset, setOffset] = useState(0);
  const [selected, setSelected] = useState('');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const path = '/assets/' + encodeURIComponent(asset.id) + '/bindings';
  const discoveries = useResource<Discoveries>(adding
    ? '/nodes/' + encodeURIComponent(asset.node_id) + '/discoveries?limit=' + pageSize + '&offset=' + offset
    : null, onExpired);
  const bound = new Set(asset.bindings.map((binding) => binding.id));
  const choices = discoveries.data?.discoveries.filter((item) => !bound.has(item.id)) ?? [];

  async function change(discoveryId: string, remove: boolean) {
    setBusy(true);
    setError('');
    try {
      await (remove
        ? request<void>(path + '/' + encodeURIComponent(discoveryId), { method: 'DELETE' })
        : request<void>(path, { method: 'POST', body: { discovery_id: discoveryId } }));
      setSelected('');
      onChanged();
    } catch (failure) {
      if (isUnauthorized(failure)) onExpired();
      else setError(errorMessage(failure));
    } finally {
      setBusy(false);
    }
  }

  function add(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (selected) void change(selected, false);
  }

  return (
    <section className="monitor-section" aria-label="运行实例">
      <div className="section-bar">
        <h2>运行实例</h2>
        <button type="button" onClick={() => setAdding(!adding)} disabled={busy}>{adding ? '关闭关联选择' : '关联实例'}</button>
      </div>
      {error && <p role="alert" className="error">{error}</p>}
      {asset.bindings.length ? (
        <DataTable label="已关联实例" columns={['实例', '类型', '来源', '最后观测', '关联时间', '操作']}>
          {asset.bindings.map((binding) => (
            <tr key={binding.id}>
              <th scope="row">{binding.name}<div className="table-secondary technical">{binding.resource_id}</div></th>
              <td>{managementLabels[binding.kind]}</td><td>Agent 发现</td>
              <td><Timestamp value={binding.observed_at} /></td><td><Timestamp value={binding.bound_at} /></td>
              <td><button className="quiet" type="button" disabled={busy} onClick={() => void change(binding.id, true)}>解除关联</button></td>
            </tr>
          ))}
        </DataTable>
      ) : <p className="empty">暂无关联实例</p>}
      {adding && (
        <div className="monitor-section detail-panel">
          {discoveries.loading && <p role="status">正在读取发现清单…</p>}
          {discoveries.error && <Problem error={discoveries.error} retry={discoveries.refresh} />}
          {discoveries.data && !choices.length && <p>本页无可关联实例 · <a className="text-link" href={'/nodes/' + encodeURIComponent(asset.node_id)}>查看节点监控</a></p>}
          {choices.length > 0 && (
            <form className="filter-bar" onSubmit={add}>
              <label>已发现实例<select value={selected} onChange={(event) => setSelected(event.target.value)} required>
                <option value="">选择实例</option>
                {choices.map((item) => (
                  <option key={item.id} value={item.id}>{managementLabels[item.kind]} · {item.name} · {item.resource_id.slice(0, 18)}</option>
                ))}
              </select></label>
              <button type="submit" className="primary" disabled={busy || !selected}>确认关联</button>
            </form>
          )}
          <Pagination
            label="发现实例分页"
            offset={offset}
            pageSize={pageSize}
            hasMore={discoveries.data?.has_more}
            disabled={busy}
            onChange={(value) => { setOffset(value); setSelected(''); }}
          />
        </div>
      )}
    </section>
  );
}
