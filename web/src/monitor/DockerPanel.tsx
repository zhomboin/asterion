import { useState } from 'react';
import { request } from '../api';
import { DataTable, Pagination, Problem } from '../ui';
import type { Docker } from './data';
import { ContainerDetail } from './ContainerDetail';
import type { NodeOperations } from '../operations/data';
import { MonitorPicker, monitorPageSize } from './MonitorPicker';
import { useManagedResources } from './managedResources';

const statusText = {
  available: '可用',
  disabled: '未启用 Docker 采集',
  not_installed: '未发现 Docker Engine',
  permission_denied: 'SSH 账号无权读取 Docker 接口',
  unreachable: 'Docker Engine 不可达',
};

export function DockerPanel({ base, online, onExpired, operations }: {
  base: string;
  online: boolean;
  onExpired: () => void;
  operations?: NodeOperations;
}) {
  const [adding, setAdding] = useState(false);
  const [selected, setSelected] = useState<number>();
  const list = useManagedResources<Docker>(base, 'docker', online, onExpired);
  const engine = list.snapshot.error ? undefined : list.snapshot.data;
  const observed = (target: string, engineId: string) => engine?.status === 'available' && engine.engine_id === engineId
    ? engine.containers.find((item) => item.id === target) : undefined;
  const selectedItem = list.rows.find((item) => item.id === selected);
  const container = selectedItem && observed(selectedItem.target, selectedItem.engine_id);

  return <section className="monitor-view" aria-label="Docker">
    <div className="section-bar">
      <h2>已纳管容器</h2>
      <button type="button" className="primary" disabled={!online} onClick={() => setAdding(true)}>添加监控</button>
    </div>
    {!online && <p className="observation">SSH 未连接，实时状态不可用</p>}
    {list.registry.loading && <p role="status">正在读取纳管清单…</p>}
    {list.registry.error && <Problem error={list.registry.error} retry={list.refresh} />}
    {list.snapshot.error && <Problem error={list.snapshot.error} retry={list.refresh} />}
    {list.error && <p className="error" role="alert">{list.error}</p>}
    {engine && engine.status !== 'available' && <p className="observation">{statusText[engine.status]}</p>}
    {list.registry.data && !list.rows.length && <p className="empty">暂无纳管容器</p>}
    {list.rows.length > 0 && <DataTable label="纳管容器" columns={['容器', '镜像', '状态 / 健康', '操作']}>
      {list.rows.map((item) => {
        const row = observed(item.target, item.engine_id);
        const missing = engine?.status === 'available' && engine.engine_id === item.engine_id && !engine.truncated;
        const differentEngine = engine?.status === 'available' && engine.engine_id && engine.engine_id !== item.engine_id;
        return <tr key={item.id}>
          <th scope="row"><div>{row?.name || item.name}</div>
            <div className="table-secondary technical" title={item.target}>{item.target.slice(0, 12)}</div></th>
          <td>{row?.image ?? '—'}</td>
          <td>{row ? <><div>{row.status || row.state}</div><div className="table-secondary">{row.health ?? '—'}</div></>
            : differentEngine ? '连接了其他 Engine' : missing ? '未发现容器' : '状态未知'}</td>
          <td><div className="row-actions">
            {row && <button type="button" className="quiet" onClick={() => setSelected(item.id)}>详情</button>}
            <button type="button" className="quiet" disabled={list.removing !== undefined} onClick={() => {
              if (selected === item.id) setSelected(undefined);
              void list.remove(item.id);
            }}>取消纳管</button>
          </div></td>
        </tr>;
      })}
    </DataTable>}
    <Pagination label="纳管容器分页" offset={list.offset} pageSize={monitorPageSize} hasMore={list.registry.data?.has_more}
      onChange={(offset) => { setSelected(undefined); list.setOffset(offset); }} />
    {container && <ContainerDetail key={container.id} base={base + '/docker/containers/' + encodeURIComponent(container.id)}
      item={container} engineId={selectedItem.engine_id} operations={operations} onClose={() => setSelected(undefined)} onExpired={onExpired} />}
    {adding && online && <MonitorPicker<Docker>
      title="添加容器监控" placeholder="名称、镜像或 ID" onClose={() => setAdding(false)} onExpired={onExpired}
      path={(q) => base + '/docker/containers' + (q ? '?' + new URLSearchParams({ q }) : '')}
      results={(data) => {
        const engineId = data.engine_id;
        if (data.status !== 'available') return { candidates: [], notice: statusText[data.status] };
        if (!engineId) return { candidates: [], notice: '无法确认 Docker Engine 标识' };
        return {
          notice: data.truncated ? '候选清单达到上限，请缩小搜索范围' : undefined,
          candidates: data.containers.map((row) => ({
            id: row.id, name: row.name || row.id.slice(0, 12), description: row.image,
            disabled: list.rows.some((item) => item.target === row.id && item.engine_id === engineId),
            add: () => request(list.path, { method: 'POST', body: { kind: 'docker', container_id: row.id, engine_id: engineId } }),
          })),
        };
      }} onAdded={() => { setAdding(false); list.refresh(); }} />}
  </section>;
}
