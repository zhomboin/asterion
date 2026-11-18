import { useState } from 'react';
import { DataTable, Problem, Timestamp } from '../ui';
import { useResource } from '../useResource';
import type { Container, Docker } from './data';
import { ContainerDetail } from './ContainerDetail';
import type { NodeOperations } from '../operations/data';

const statusText = {
  available: '可用',
  disabled: '未启用 Docker 采集',
  not_installed: '未发现 Docker Engine',
  permission_denied: 'Agent 无权读取 Docker 接口',
  unreachable: 'Docker Engine 不可达',
};

export function DockerPanel({ base, onExpired, operations }: {
  base: string;
  onExpired: () => void;
  operations?: NodeOperations;
}) {
  const inventory = useResource<Docker>(base + '/docker/containers', onExpired);
  const [query, setQuery] = useState('');
  const [selected, setSelected] = useState<string>();
  const engine = inventory.data;
  const container = engine?.containers.find((item) => item.id === selected);
  const search = query.trim().toLocaleLowerCase();
  const rows = engine?.containers.filter((item) => matches(item, search)) ?? [];

  return (
    <section className="monitor-view" aria-label="Docker">
      <div className="section-bar">
        <h2>Docker</h2>
        <button type="button" onClick={inventory.refresh}>刷新</button>
      </div>
      {inventory.loading && <p role="status">正在读取 Docker Engine…</p>}
      {inventory.error && <Problem error={inventory.error} retry={inventory.refresh} />}
      {engine?.status !== undefined && engine.status !== 'available' && <p className="empty">{statusText[engine.status]}</p>}
      {engine?.status === 'available' && (
        <>
          <dl className="detail-facts">
            <div><dt>Engine 版本</dt><dd>{engine.engine_version ?? '—'}</dd></div>
            <div><dt>Engine ID</dt><dd className="technical">{engine.engine_id ?? '—'}</dd></div>
            <div><dt>连接指纹</dt><dd className="technical">{engine.endpoint_fingerprint ?? '—'}</dd></div>
          </dl>
          <p className="observation">来源 · Docker Engine</p>
          <label className="search-field">搜索容器
            <input type="search" value={query} onChange={(event) => setQuery(event.target.value)} placeholder="名称、镜像或 ID" />
          </label>
          {engine.truncated && <p className="observation">当前容器清单达到显示上限</p>}
          {rows.length ? (
            <DataTable label="容器列表" columns={['容器', '镜像', '状态 / 健康', '启动时间', '重启', '操作']}>
              {rows.map((item) => (
                <tr key={item.id}>
                  <th scope="row">
                    <div>{item.name}</div>
                    <div className="table-secondary technical" title={item.id}>{item.id.slice(0, 12)}</div>
                  </th>
                  <td>{item.image}</td>
                  <td><div>{item.status || item.state}</div><div className="table-secondary">{item.health ?? '—'}</div></td>
                  <td><Timestamp value={item.started_at} /></td>
                  <td>{item.restart_count ?? '—'}</td>
                  <td><button className="quiet" type="button" aria-label={'查看容器 ' + item.name} onClick={() => setSelected(item.id)}>详情</button></td>
                </tr>
              ))}
            </DataTable>
          ) : <p className="empty">{search ? '没有匹配的容器' : '暂无容器'}</p>}
          {selected && !container && <p className="observation" role="status">该容器已不在最新清单中</p>}
          {container && (
            <ContainerDetail
              key={container.id}
              base={base + '/docker/containers/' + encodeURIComponent(container.id)}
              item={container}
              engineId={engine.engine_id}
              operations={operations}
              onClose={() => setSelected(undefined)}
              onExpired={onExpired}
            />
          )}
        </>
      )}
    </section>
  );
}

function matches(item: Container, query: string) {
  return [item.name, item.id, item.image, item.compose_project ?? '', item.compose_service ?? '']
    .some((value) => value.toLocaleLowerCase().includes(query));
}
