import { useState, type FormEvent } from 'react';
import { DataTable, Problem } from '../ui';
import { useResource } from '../useResource';
import type { NodeOperations } from '../operations/data';
import type { Account, Keys } from './data';

export function AccountDetail({ base, user, onClose, onExpired, operations }: {
  base: string;
  user: Account;
  onClose: () => void;
  onExpired: () => void;
  operations?: NodeOperations;
}) {
  const keys = useResource<Keys>(base + '/keys', onExpired);
  const [editing, setEditing] = useState<'key' | 'groups'>();
  const groups = operations?.catalog.accounts.groups ?? [];

  function setGroups(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const selected = new FormData(event.currentTarget).getAll('groups').map(String);
    operations?.propose({ type: 'account', change: { type: 'set_groups', username: user.username, groups: selected } });
  }

  function addKey(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const key = String(new FormData(event.currentTarget).get('key') ?? '').trim();
    if (key) operations?.propose({ type: 'account', change: { type: 'add_key', username: user.username, key } });
  }

  return (
    <section className="monitor-section detail-panel" aria-label="账号详情">
      <div className="section-bar">
        <h2>{user.username}</h2>
        <div className="actions">
          {operations && groups.length > 0 && <button type="button" onClick={() => setEditing('groups')}>编辑附加组</button>}
          <button type="button" className="quiet" onClick={onClose}>关闭</button>
        </div>
      </div>
      <dl className="detail-facts">
        <div><dt>账号类型</dt><dd>{user.system ? '系统账号' : '普通账号'}</dd></div>
        <div><dt>UID / GID</dt><dd>{user.uid} / {user.gid}</dd></div>
        <div><dt>主目录</dt><dd className="technical">{user.home || '—'}</dd></div>
        <div><dt>Shell</dt><dd className="technical">{user.shell || '—'}</dd></div>
        <div><dt>用户组</dt><dd>{user.groups.join('、') || '—'}</dd></div>
      </dl>
      {editing === 'groups' && operations && (
        <form className="fields" onSubmit={setGroups}>
          <fieldset className="group-options">
            <legend>授权附加组</legend>
            {groups.map((group) => (
              <label className="checkbox-field" key={group}><input type="checkbox" name="groups" value={group} defaultChecked={user.groups.includes(group)} />{group}</label>
            ))}
          </fieldset>
          <div className="dialog-actions"><button type="button" onClick={() => setEditing(undefined)}>取消</button><button type="submit">检查操作</button></div>
        </form>
      )}
      <div className="section-bar monitor-section">
        <h3>SSH 公钥</h3>
        <div className="actions">
          <button type="button" onClick={keys.refresh}>刷新</button>
          {operations && <button type="button" onClick={() => setEditing('key')}>添加公钥</button>}
        </div>
      </div>
      {editing === 'key' && operations && (
        <form className="fields" onSubmit={addKey}>
          <label>OpenSSH 公钥<input name="key" spellCheck={false} autoComplete="off" maxLength={8192} required /></label>
          <div className="dialog-actions"><button type="button" onClick={() => setEditing(undefined)}>取消</button><button type="submit">检查操作</button></div>
        </form>
      )}
      {keys.loading && <p role="status">正在读取公钥元数据…</p>}
      {keys.error && <Problem error={keys.error} retry={keys.refresh} />}
      {keys.data?.truncated && <p className="observation">当前公钥清单达到显示上限</p>}
      {keys.data && !keys.data.keys.length && <p className="empty">暂无已授权公钥</p>}
      {Boolean(keys.data?.keys.length) && (
        <DataTable label="SSH 公钥" columns={operations ? ['类型', '指纹', '备注', '操作'] : ['类型', '指纹', '备注']}>
          {keys.data?.keys.map((key, index) => (
            <tr key={index}>
              <th scope="row">{key.type}</th><td className="technical">{key.fingerprint}</td><td>{key.comment || '—'}</td>
              {operations && <td><button type="button" className="danger quiet" onClick={() => {
                operations.propose({ type: 'account', change: { type: 'remove_key', username: user.username, fingerprint: key.fingerprint } });
              }}>移除公钥</button></td>}
            </tr>
          ))}
        </DataTable>
      )}
    </section>
  );
}
