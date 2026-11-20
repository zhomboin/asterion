import { useState } from 'react';
import { request } from '../api';
import { bytes, DataTable, fraction, Pagination, Problem, Timestamp } from '../ui';
import { useResource } from '../useResource';
import type { Process, ProcessDetail } from './data';
import type { NodeOperations, ProcessSignal } from '../operations/data';
import { MonitorPicker, monitorPageSize } from './MonitorPicker';
import { useManagedResources, type ManagedResource } from './managedResources';

const states: Record<string, string> = {
  R: '运行', S: '休眠', D: '等待 I/O', Z: '僵尸', T: '停止', t: '跟踪停止', I: '空闲', X: '退出',
};

export function ProcessesPanel({ base, online, onExpired, operations }: {
  base: string;
  online: boolean;
  onExpired: () => void;
  operations?: NodeOperations;
}) {
  const [adding, setAdding] = useState(false);
  const [selected, setSelected] = useState<ManagedResource>();
  const [instance, setInstance] = useState<Process>();
  const list = useManagedResources<Process[]>(base, 'process', online, onExpired);
  const visible = list.snapshot.error ? undefined : list.snapshot.data;
  const matches = (target: string) => visible?.filter((item) => item.name === target) ?? [];
  const detail = useResource<ProcessDetail>(online && instance ? base + '/processes/' + instance.pid : null, onExpired);
  const detailsMatch = instance && detail.data?.start_ticks === instance.start_ticks && detail.data.name === instance.name;

  return <section className="monitor-view" aria-label="进程">
    <div className="section-bar">
      <h2>已纳管进程</h2>
      <button type="button" className="primary" disabled={!online} onClick={() => setAdding(true)}>添加监控</button>
    </div>
    {!online && <p className="observation">SSH 未连接，实时状态不可用</p>}
    {list.registry.loading && <p role="status">正在读取纳管清单…</p>}
    {list.registry.error && <Problem error={list.registry.error} retry={list.refresh} />}
    {list.snapshot.error && <Problem error={list.snapshot.error} retry={list.refresh} />}
    {list.error && <p className="error" role="alert">{list.error}</p>}
    {list.registry.data && !list.rows.length && <p className="empty">暂无纳管进程</p>}
    {list.rows.length > 0 && <DataTable label="纳管进程" columns={['进程名称', '当前实例', '操作']}>
      {list.rows.map((item) => <tr key={item.id}>
        <th scope="row" className="technical">{item.name}</th>
        <td>{!visible ? '状态未知' : matches(item.target).length ? '已观测到' : '未观测到'}</td>
        <td><div className="row-actions">
          <button type="button" className="quiet" onClick={() => { setSelected(item); setInstance(undefined); }}>详情</button>
          <button type="button" className="quiet" disabled={list.removing !== undefined} onClick={() => {
            if (selected?.id === item.id) { setSelected(undefined); setInstance(undefined); }
            void list.remove(item.id);
          }}>取消纳管</button>
        </div></td>
      </tr>)}
    </DataTable>}
    <Pagination label="纳管进程分页" offset={list.offset} pageSize={monitorPageSize} hasMore={list.registry.data?.has_more} onChange={(offset) => {
      setSelected(undefined); setInstance(undefined); list.setOffset(offset);
    }} />
    {selected && list.rows.some((item) => item.id === selected.id) && <section className="monitor-section detail-panel" aria-label="进程详情">
      <div className="section-bar"><h3 className="technical">{selected.name}</h3>
        <button type="button" className="quiet" onClick={() => { setSelected(undefined); setInstance(undefined); }}>关闭</button>
      </div>
      {visible && !matches(selected.target).length && <p className="empty">未观测到同名进程</p>}
      {matches(selected.target).length > monitorPageSize && <p className="observation">仅显示前 12 个匹配实例</p>}
      <ProcessInstances key={selected.id} rows={matches(selected.target).slice(0, monitorPageSize)} onSelect={setInstance} />
      {detail.loading && <p role="status">正在读取进程详情…</p>}
      {detail.error && <Problem error={detail.error} retry={detail.refresh} />}
      {detail.data && (detailsMatch
        ? <ProcessFacts item={detail.data} operations={operations} />
        : <p role="status">该进程实例已变化</p>)}
    </section>}
    {adding && online && <MonitorPicker<Process[]>
      title="添加进程监控" placeholder="名称或 PID" remotePaging onClose={() => setAdding(false)} onExpired={onExpired}
      path={(q, offset) => base + '/processes?' + new URLSearchParams({ q, sort: 'pid', limit: String(monitorPageSize + 1), offset: String(offset) })}
      results={(data) => ({ hasMore: data.length > monitorPageSize, candidates: data.map((row) => ({
        id: String(row.pid), name: row.name, description: 'PID ' + row.pid + ' · 按精确名称匹配同名进程',
        disabled: list.rows.some((item) => item.target === row.name),
        add: () => request(list.path, { method: 'POST', body: { kind: 'process', pid: row.pid, start_ticks: row.start_ticks } }),
      })) })}
      onAdded={() => { setAdding(false); list.refresh(); }} />}
  </section>;
}

function ProcessInstances({ rows, onSelect }: { rows: Process[]; onSelect: (row: Process) => void }) {
  if (!rows.length) return null;
  return <DataTable label="进程实例" columns={['PID', '状态', 'CPU', '内存', '启动时间', '操作']}>
      {rows.map((row) => <tr key={row.pid + ':' + row.start_ticks}>
        <th scope="row" className="technical">{row.pid}</th><td>{states[row.state] ?? row.state}</td>
        <td>{fraction(row.cpu_percent)}</td><td>{bytes(row.memory_bytes)}</td><td><Timestamp value={row.started_at} /></td>
        <td><button type="button" className="quiet" aria-label={'查看进程 ' + row.pid} onClick={() => onSelect(row)}>详情</button></td>
      </tr>)}
    </DataTable>;
}

function ProcessFacts({ item, operations }: { item: ProcessDetail; operations?: NodeOperations }) {
  function propose(signal: ProcessSignal) {
    operations?.propose({ type: 'process', pid: item.pid, start_ticks: item.start_ticks, signal }, item.name);
  }
  const policy = operations?.catalog.process;
  return (
    <>
      <dl className="detail-facts">
        <div><dt>名称</dt><dd>{item.name}</dd></div>
        <div><dt>父进程</dt><dd>{item.parent_pid ?? '不可用'}</dd></div>
        <div><dt>账号</dt><dd>{item.user ?? '不可用'}</dd></div>
        <div><dt>运行归属</dt><dd className="technical">{runtimeLabel(item.runtime)}</dd></div>
        <div><dt>可执行文件</dt><dd className="technical">{item.executable ?? '不可用'}</dd></div>
      </dl>
      <h3>进程参数</h3>
      {item.command_line?.length ? (
        <ul className="argument-list">{item.command_line.map((arg, index) => <li key={index}>{arg}</li>)}</ul>
      ) : <p>{item.command_line == null ? '不可用' : '无参数'}</p>}
      {policy && item.pid > 1 && item.start_ticks > 0 && (policy.term || policy.kill) && (
        <div className="actions service-actions" aria-label="进程运维">
          {policy.term && <button type="button" onClick={() => propose('term')}>终止进程</button>}
          {policy.kill && <button type="button" className="danger" onClick={() => propose('kill')}>强制终止</button>}
        </div>
      )}
    </>
  );
}

function runtimeLabel(runtime: ProcessDetail['runtime']) {
  switch (runtime?.kind) {
    case 'systemd': return runtime.unit;
    case 'docker': return 'Docker · ' + runtime.container_id;
    default: return '未知';
  }
}
