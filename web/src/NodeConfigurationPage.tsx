import { useState, type FormEvent } from 'react';
import { errorMessage, isUnauthorized, request, type Node } from './api';
import { useResource } from './useResource';
import { PageHeader, Problem } from './ui';

export function NodeConfigurationPage({ id, onExpired }: { id?: string; onExpired: () => void }) {
  const resource = useResource<Node>(id ? '/nodes/' + id : null, onExpired);
  const profiles = useResource<string[]>('/ssh/profiles', onExpired);
  const settings = useResource<{ local_ssh: boolean }>('/settings', onExpired);
  const local = settings.data?.local_ssh;
  const [selected, setSelected] = useState('');
  const node = resource.data;
  const [removing, setRemoving] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');

  async function save(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const data = new FormData(event.currentTarget);
    setBusy(true);
    setError('');
    try {
      const path = id ? '/nodes/' + id : '/nodes';
      if (removing) {
        await request(path, { method: 'DELETE', response: 'empty' });
        window.location.assign('/nodes');
      } else {
        const address = String(data.get('destination') ?? '').trim();
        const connection = node?.ssh_profile ?? (local && !selected ? 'ssh://' + address : selected);
        const body = {
          name: String(data.get('name') ?? '').trim() || (address || selected).slice(0, 100), kind: data.get('kind'), host_name: data.get('host_name') || null,
          tags: String(data.get('tags')).split(/\r?\n/u).map(t => t.trim()).filter(Boolean),
          notes: data.get('notes'), ssh_profile: connection,
        };
        const saved = await request<Node>(path, { method: id ? 'PATCH' : 'POST', body });
        window.location.assign('/nodes/' + saved.id);
      }
    } catch (failure) {
      if (isUnauthorized(failure)) onExpired();
      else setError(errorMessage(failure));
    } finally {
      setBusy(false);
    }
  }

  return <>
    <PageHeader title={id ? '管理节点' : '通过 SSH 接入节点'} back={{ href: '/nodes', label: '节点列表' }} />
    {resource.error && <Problem error={resource.error} retry={resource.refresh} />}
    {profiles.error && <Problem error={profiles.error} retry={profiles.refresh} />}
    {settings.error && <Problem error={settings.error} retry={settings.refresh} />}
    {settings.data && (!id || node) && <section className="node-configuration stage-card">
      <form className="fields" onSubmit={save}>
        {removing ? <p>确认移除节点「{node?.name}」及其监控数据？</p> : <>
          {node?.ssh_profile ? <label>SSH 连接<input readOnly value={node.ssh_profile.replace(/^ssh:\/\//u, '')} /></label> : <>
            {(!local || !!profiles.data?.length) && <label>SSH 连接<select required={!local}
              value={selected} onChange={event => setSelected(event.target.value)}>
              <option value="">{local ? '直接连接' : '选择中心已配置的连接'}</option>
              {profiles.data?.map(profile => <option key={profile}>{profile}</option>)}
            </select></label>}
            {local && !selected && <label>SSH 地址或别名<input name="destination" required maxLength={330}
              placeholder="用户@主机、用户@主机:端口 或 SSH 别名" autoCapitalize="none" spellCheck={false} /></label>}
            {!local && profiles.data?.length === 0 && <p role="status">请先在中心的 SSH 配置文件中添加连接。</p>}
          </>}
          <label>节点名称<input name="name" defaultValue={node?.name} required={!local || !!id} placeholder={local && !id ? '默认使用 SSH 连接名' : undefined} maxLength={100} /></label>
          <label>环境<select name="kind" defaultValue={node?.kind ?? 'linux'}>
            <option value="linux">Linux</option><option value="wsl">WSL</option>
          </select></label>
          <label>宿主机标识<input name="host_name" defaultValue={node?.host_name ?? ''} maxLength={100} /></label>
          <label>标签<textarea name="tags" defaultValue={node?.tags.join('\n')} placeholder="每行一个标签" rows={3} /></label>
          <label>备注<textarea name="notes" defaultValue={node?.notes} rows={3} maxLength={1000} /></label>
        </>}
        {error && <p className="error" role="alert">{error}</p>}
        <div className="dialog-actions">
          {id && !removing && <button type="button" className="danger quiet" onClick={() => setRemoving(true)}>移除节点</button>}
          {removing && <button type="button" onClick={() => setRemoving(false)} disabled={busy}>返回</button>}
          <button className={removing ? 'primary danger' : 'primary'} type="submit" disabled={busy}>
            {removing ? '确认移除' : id ? '保存' : '接入节点'}
          </button>
        </div>
      </form>
    </section>}
  </>;
}
