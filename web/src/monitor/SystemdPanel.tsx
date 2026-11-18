import { useState, type FormEvent } from 'react';
import { DataTable, Problem, Timestamp } from '../ui';
import { useResource } from '../useResource';
import type { Systemd, Unit } from './data';
import { ServiceActions } from '../operations/ServiceActions';
import type { NodeOperations } from '../operations/data';

const statusText = {
  available: '可用',
  unavailable: 'systemd 未启用或不可用',
  permission_denied: 'Agent 无权读取 systemd',
  error: 'systemd 查询失败',
};

export function SystemdPanel({ base, onExpired, operations }: {
  base: string;
  onExpired: () => void;
  operations?: NodeOperations;
}) {
  const [query, setQuery] = useState('');
  const [selected, setSelected] = useState<string>();
  const path = base + '/systemd/services';
  const inventory = useResource<Systemd>(query ? path + '?' + new URLSearchParams({ q: query }) : path, onExpired);
  const detail = useResource<Unit>(selected ? path + '/' + encodeURIComponent(selected) : null, onExpired);
  const systemd = inventory.data;
  const rows = systemd?.units ?? [];

  function search(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setQuery(String(new FormData(event.currentTarget).get('query') ?? '').trim());
  }

  return (
    <section className="monitor-view" aria-label="systemd">
      <div className="section-bar">
        <h2>systemd</h2>
        <button type="button" onClick={inventory.refresh}>刷新</button>
      </div>
      {(!systemd || systemd.status === 'available') && (
        <form className="filter-bar operation-filters" onSubmit={search}>
          <label>搜索 Unit<input name="query" type="search" placeholder="Unit 名称" maxLength={192} /></label>
          <button type="submit">查询</button>
        </form>
      )}
      {inventory.loading && <p role="status">正在读取 systemd 服务…</p>}
      {inventory.error && <Problem error={inventory.error} retry={inventory.refresh} />}
      {systemd && systemd.status !== 'available' && <p className="empty">{statusText[systemd.status]}</p>}
      {systemd?.status === 'available' && (
        <>

          {systemd.truncated && <p className="observation">当前服务清单达到显示上限</p>}
          {rows.length ? (
            <DataTable label="systemd 服务列表" columns={['Unit', 'Active', 'Sub', '描述', '操作']}>
              {rows.map((item) => (
                <tr key={item.name}>
                  <th scope="row" className="technical">{item.name}</th>
                  <td>{item.active}</td><td>{item.sub}</td><td>{item.description || '—'}</td>
                  <td><button type="button" className="quiet" aria-label={'查看 ' + item.name} onClick={() => setSelected(item.name)}>详情</button></td>
                </tr>
              ))}
            </DataTable>
          ) : <p className="empty">{query ? '没有匹配的 Unit' : '暂无服务 Unit'}</p>}
        </>
      )}
      {selected && (
        <section className="monitor-section detail-panel" aria-label="服务详情">
          <div className="section-bar">
            <h2 className="technical">{selected}</h2>
            <button type="button" className="quiet" onClick={() => setSelected(undefined)}>关闭</button>
          </div>
          {detail.loading && <p role="status">正在读取 Unit 属性…</p>}
          {detail.error && <Problem error={detail.error} retry={detail.refresh} />}
          {detail.data && <UnitFacts unit={detail.data} />}
          {detail.data && operations?.catalog.systemd_units.includes(selected) && (
            <ServiceActions title="systemd 运维" onChoose={(verb) => {
              operations.propose({ type: 'systemd', unit: selected, verb });
            }} />
          )}
        </section>
      )}
    </section>
  );
}

function UnitFacts({ unit }: { unit: Unit }) {
  return (
    <>
      <p className="observation">
        来源 · {unit.source} · 观测 <Timestamp value={unit.observed_at} /> · {unit.verified ? '已核实' : '未核实'}
      </p>
      <dl className="detail-facts asset-facts">
        <div><dt>Active / Sub</dt><dd>{unit.active} / {unit.sub}</dd></div>
        <div><dt>ExecStart</dt><dd className="technical">{unit.exec_start ?? '不可用'}</dd></div>
        <div><dt>WorkingDirectory</dt><dd className="technical">{unit.working_directory ?? '不可用'}</dd></div>
        <div><dt>FragmentPath</dt><dd className="technical">{unit.fragment_path ?? '不可用'}</dd></div>
      </dl>
    </>
  );
}
