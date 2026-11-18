import { useState, type FormEvent } from 'react';
import { bytes, DataTable, fraction, Pagination, Problem, Timestamp } from '../ui';
import { useResource } from '../useResource';
import type { Process, ProcessDetail } from './data';
import type { NodeOperations, ProcessSignal } from '../operations/data';

const states: Record<string, string> = {
  R: '运行', S: '休眠', D: '等待 I/O', Z: '僵尸', T: '停止', t: '跟踪停止', I: '空闲', X: '退出',
};
const pageSize = 50;
const sortItems = [
  ['cpu', 'CPU 降序'],
  ['memory', '内存降序'],
  ['pid', 'PID 升序'],
  ['name', '名称升序'],
];

export function ProcessesPanel({ base, onExpired, operations }: {
  base: string;
  onExpired: () => void;
  operations?: NodeOperations;
}) {
  const [query, setQuery] = useState('');
  const [sort, setSort] = useState('cpu');
  const [offset, setOffset] = useState(0);
  const [pid, setPid] = useState<number>();
  const params = new URLSearchParams({ sort, limit: String(pageSize + 1), offset: String(offset) });
  if (query) params.set('q', query);
  const list = useResource<Process[]>(base + '/processes?' + params, onExpired);
  const detail = useResource<ProcessDetail>(pid === undefined ? null : base + '/processes/' + pid, onExpired);

  function search(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setQuery(String(new FormData(event.currentTarget).get('query') ?? '').trim());
    setOffset(0);
  }

  return (
    <section aria-label="进程" className="monitor-view">
      <div className="section-bar">
        <h2>进程</h2>
        <button type="button" onClick={list.refresh}>刷新</button>
      </div>
      <form className="filter-bar" onSubmit={search}>
        <label>搜索<input name="query" type="search" placeholder="名称或 PID" maxLength={80} /></label>
        <label>排序<select value={sort} onChange={(event) => { setSort(event.target.value); setOffset(0); }}>
          {sortItems.map(([value, label]) => <option key={value} value={value}>{label}</option>)}
        </select></label>
        <button type="submit">查询</button>
      </form>
      {list.loading && <p role="status">正在读取进程…</p>}
      {list.error && <Problem error={list.error} retry={list.refresh} />}
      {list.data && list.data.length === 0 && <p className="empty">没有匹配的进程</p>}
      {Boolean(list.data?.length) && (
        <DataTable label="进程列表" columns={['PID', '名称', '状态', 'CPU', '内存', '启动时间', '操作']}>
          {list.data?.slice(0, pageSize).map((row) => (
            <tr key={row.pid}>
              <td className="technical">{row.pid}</td><th scope="row">{row.name}</th>
              <td>{states[row.state] ?? (row.state || '—')}</td><td>{fraction(row.cpu_percent)}</td>
              <td>{bytes(row.memory_bytes)}</td><td><Timestamp value={row.started_at} /></td>
              <td><button type="button" className="quiet" aria-label={'查看进程 ' + row.pid} onClick={() => setPid(row.pid)}>详情</button></td>
            </tr>
          ))}
        </DataTable>
      )}
      <Pagination
        label="进程分页"
        offset={offset}
        pageSize={pageSize}
        hasMore={(list.data?.length ?? 0) > pageSize}
        maxOffset={10_000}
        onChange={setOffset}
      />
      {pid !== undefined && (
        <section className="monitor-section detail-panel" aria-label="进程详情">
          <div className="section-bar">
            <h2>进程 {pid}</h2>
            <button className="quiet" type="button" onClick={() => setPid(undefined)}>关闭</button>
          </div>
          {detail.loading && <p role="status">正在读取进程详情…</p>}
          {detail.error && <Problem error={detail.error} retry={detail.refresh} />}
          {detail.data && <ProcessFacts item={detail.data} operations={operations} />}
        </section>
      )}
    </section>
  );
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
