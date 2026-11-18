import { useEffect, useRef, useState, type FormEvent } from 'react';
import { errorMessage, isUnauthorized } from '../api';
import { DataTable, Pagination, Problem } from '../ui';
import { useResource } from '../useResource';

export const monitorPageSize = 12;

export function useMonitorPage(total: number) {
  const [cursor, setOffset] = useState(0);
  const offset = Math.min(cursor, Math.floor(Math.max(0, total - 1) / monitorPageSize) * monitorPageSize);
  if (cursor !== offset) setOffset(offset);
  return { offset, setOffset, hasMore: offset + monitorPageSize < total };
}

type Candidate = { id: string; name: string; description: string; disabled?: boolean; add: () => Promise<unknown> };

type PickerProps<T> = {
  title: string;
  placeholder: string;
  path: (query: string, offset: number) => string;
  results: (data: T) => { candidates: Candidate[]; notice?: string; hasMore?: boolean };
  remotePaging?: boolean;
  onClose: () => void;
  onAdded: (id: string) => void;
  onExpired: () => void;
};

export function MonitorPicker<T>({ title, placeholder, path, results, remotePaging, onClose, onAdded, onExpired }: PickerProps<T>) {
  const ref = useRef<HTMLDialogElement>(null);
  const [query, setQuery] = useState('');
  const [cursor, setOffset] = useState(0);
  const [busy, setBusy] = useState<string>();
  const [error, setError] = useState('');
  const source = useResource<T>(path(query, remotePaging ? cursor : 0), onExpired);
  const result = source.data === undefined ? undefined : results(source.data);
  const entries = result?.candidates ?? [];
  const offset = remotePaging ? cursor : Math.min(cursor, Math.floor(Math.max(0, entries.length - 1) / monitorPageSize) * monitorPageSize);
  const rows = remotePaging ? entries.slice(0, monitorPageSize) : entries.slice(offset, offset + monitorPageSize);

  useEffect(() => {
    const dialog = ref.current;
    dialog?.showModal();
    return () => dialog?.close();
  }, []);

  function search(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setError('');
    setQuery(String(new FormData(event.currentTarget).get('q') ?? '').trim());
    setOffset(0);
    source.refresh();
  }

  async function add(item: Candidate) {
    setBusy(item.id);
    setError('');
    try {
      await item.add();
      onAdded(item.id);
    } catch (failure) {
      if (isUnauthorized(failure)) onExpired();
      else setError(errorMessage(failure));
    } finally {
      setBusy(undefined);
    }
  }

  return <dialog ref={ref} className="monitor-picker" aria-labelledby="monitor-picker-title" onCancel={(event) => {
    event.preventDefault();
    if (!busy) onClose();
  }}>
    <div className="section-bar">
      <h2 id="monitor-picker-title">{title}</h2>
      <button type="button" disabled={Boolean(busy)} onClick={onClose}>关闭</button>
    </div>
    <form className="filter-bar" onSubmit={search}>
      <label>搜索<input name="q" type="search" placeholder={placeholder} maxLength={80} /></label>
      <button type="submit" disabled={Boolean(busy)}>查询</button>
    </form>
    {source.loading && <p role="status">正在发现监控对象…</p>}
    {source.error && <Problem error={source.error} retry={source.refresh} />}
    {error && <p className="error" role="alert">{error}</p>}
    {result?.notice && <p className="observation">{result.notice}</p>}
    {result && !rows.length && !result.notice && <p className="empty">没有匹配的对象</p>}
    {rows.length > 0 && <DataTable label={title + '候选列表'} columns={['对象', '操作']}>
      {rows.map((item) => <tr key={item.id}>
        <th scope="row"><span className="technical">{item.name}</span><div className="table-secondary">{item.description}</div></th>
        <td><button type="button" className="quiet" aria-label={(item.disabled ? '已纳管 ' : '纳管 ') + item.name}
          disabled={Boolean(busy || item.disabled || source.error)} onClick={() => void add(item)}>
          {item.disabled ? '已纳管' : busy === item.id ? '正在保存…' : '纳管'}
        </button></td>
      </tr>)}
    </DataTable>}
    <Pagination label="候选对象分页" offset={offset} pageSize={monitorPageSize}
      hasMore={remotePaging ? result?.hasMore : offset + monitorPageSize < entries.length}
      maxOffset={remotePaging ? 8192 : undefined} disabled={Boolean(busy || source.loading)} onChange={setOffset} />
  </dialog>;
}
