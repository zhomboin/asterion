import { useState, type FormEvent } from 'react';
import { DataTable, Problem } from '../ui';
import { useResource } from '../useResource';
import type { NodeOperations } from '../operations/data';
import { AccountDetail } from './AccountDetail';
import type { Accounts } from './data';

export function AccountsPanel({ base, onExpired, operations }: {
  base: string;
  onExpired: () => void;
  operations?: NodeOperations;
}) {
  const list = useResource<Accounts>(base + '/accounts', onExpired);
  const [selection, setSelection] = useState<string>();
  const [query, setQuery] = useState('');
  const [creating, setCreating] = useState(false);
  const users = list.data?.users ?? [];
  const detail = users.find((user) => user.username === selection);
  const term = query.trim().toLocaleLowerCase();
  const visible = users.filter((user) => user.username.toLocaleLowerCase().includes(term) || String(user.uid) === term);
  const policy = operations?.catalog.accounts;
  const newAccounts = policy?.enabled
    ? policy.usernames.filter((username) => username !== 'root' && !users.some((user) => user.username === username))
    : [];

  function create(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const username = String(new FormData(event.currentTarget).get('username') ?? '');
    operations?.propose({ type: 'account', change: { type: 'create', username } });
  }

  return (
    <section className="monitor-view" aria-label="Linux 账号">
      <div className="section-bar">
        <h2>Linux 账号</h2>
        <div className="actions">
          <button type="button" onClick={list.refresh}>刷新</button>
          {list.data && newAccounts.length > 0 && (
            <button type="button" onClick={() => setCreating(!creating)}>{creating ? '取消' : '创建普通账号'}</button>
          )}
        </div>
      </div>
      {creating && newAccounts.length > 0 && (
        <form className="filter-bar operation-filters" onSubmit={create}>
          <label>批准的账号<select name="username" required defaultValue="">
            <option value="">选择账号</option>
            {newAccounts.map((username) => <option key={username}>{username}</option>)}
          </select></label>
          <button type="submit" className="primary">检查操作</button>
        </form>
      )}
      {list.loading && <p role="status">正在读取账号…</p>}
      {list.error && <Problem error={list.error} retry={list.refresh} />}
      {list.data && (
        <label className="search-field">搜索账号<input type="search" placeholder="账号名或 UID"
          value={query} onChange={(event) => setQuery(event.target.value)} /></label>
      )}
      {list.data?.truncated && <p className="observation">当前账号清单达到显示上限</p>}
      {list.data && !visible.length && <p className="empty">{term ? '没有匹配的账号' : '暂无可读取的账号'}</p>}
      {visible.length > 0 && (
        <DataTable label="Linux 账号" columns={['账号', 'UID', 'GID', 'Shell', '用户组', '操作']}>
          {visible.map((user) => (
            <tr key={user.username}>
              <th scope="row">{user.username}{user.system && <div className="table-secondary">系统账号</div>}</th>
              <td>{user.uid}</td><td>{user.gid}</td><td className="technical">{user.shell}</td>
              <td>{user.groups.join('、') || '—'}</td>
              <td><button type="button" className="quiet" aria-label={'查看账号 ' + user.username} onClick={() => setSelection(user.username)}>详情</button></td>
            </tr>
          ))}
        </DataTable>
      )}
      {detail && (
        <AccountDetail
          key={detail.username}
          base={base + '/accounts/' + encodeURIComponent(detail.username)}
          user={detail}
          onClose={() => setSelection(undefined)}
          onExpired={onExpired}
          operations={!detail.system && detail.username !== 'root' && policy?.enabled && policy.usernames.includes(detail.username) ? operations : undefined}
        />
      )}
    </section>
  );
}
