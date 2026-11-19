import { useState, type FormEvent } from 'react';
import {
  errorMessage, isUnauthorized, request,
  type Enrollment, type Node, type NodeDraft,
} from './api';
import { AgentEnrollment } from './AgentEnrollment';
import { SshEnrollment } from './SshEnrollment';
import { useResource } from './useResource';
import { PageHeader, Problem } from './ui';

type Props = {
  id?: string;
  deploy: boolean;
  onExpired: () => void;
};

type Step = 'form' | 'manual' | 'rotate' | 'revoke' | 'remove' | 'token';

export function NodeConfigurationPage({ id, deploy, onExpired }: Props) {
  const resource = useResource<Node>(id ? '/nodes/' + id : null, onExpired);
  const node = resource.data;
  const [step, setStep] = useState<Step>('form');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [enrollment, setEnrollment] = useState<Enrollment>();
  const onClose = () => window.location.assign('/nodes');

  async function perform(action: () => Promise<void>) {
    setBusy(true);
    setError('');
    try {
      await action();
    } catch (failure) {
      if (isUnauthorized(failure)) onExpired();
      else setError(errorMessage(failure));
    } finally {
      setBusy(false);
    }
  }

  async function save(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const data = new FormData(event.currentTarget);
    const body: NodeDraft = {
      name: String(data.get('name')).trim(),
      host_name: String(data.get('host_name') ?? '').trim() || null,
      kind: data.get('kind') === 'wsl' ? 'wsl' : 'linux',
      tags: [...new Set(String(data.get('tags')).split(/\r?\n/u).map((tag) => tag.trim()).filter(Boolean))],
      notes: String(data.get('notes')).trim(),
    };
    await perform(async () => {
      if (node) {
        await request<Node>('/nodes/' + node.id, { method: 'PATCH', body });
        onClose();
      } else {
        setEnrollment(await request<Enrollment>('/nodes/enrollment-token', { method: 'POST', body }));
        setStep('token');
      }
    });
  }

  async function confirm() {
    if (!node) return;
    const path = '/nodes/' + node.id;
    await perform(async () => {
      if (step === 'rotate') {
        setEnrollment(await request<Enrollment>(path + '/enrollment-token', { method: 'POST' }));
        setStep('token');
      } else {
        const target = step === 'remove' ? path : path + '/credentials';
        await request<void>(target, { method: 'DELETE' });
        onClose();
      }
    });
  }

  function goTo(next: Step) {
    setError('');
    setStep(next);
  }

  const titles = {
    form: deploy ? id ? 'Agent 接入' : '接入服务器' : '管理节点',
    manual: '手动接入',
    rotate: '换发注册凭证',
    revoke: '吊销凭证',
    remove: '移除节点',
    token: 'Agent 接入配置',
  };

  return (
    <>
      <PageHeader back={{ href: '/nodes', label: '返回节点列表' }} title={titles[step]} scope={node?.name} />
      {resource.error && <Problem error={resource.error} retry={resource.refresh} />}
      {resource.loading && <p role="status">正在加载节点…</p>}
      {(!id || node) && <section className="node-configuration stage-card">
        {step === 'form' && deploy && (
          <SshEnrollment existing={node} onManual={() => goTo(node ? 'rotate' : 'manual')} onExpired={onExpired} />
        )}
        {(step === 'manual' || (step === 'form' && !deploy)) && (
          <form className="fields" onSubmit={save}>
            <label>节点名称<input name="name" defaultValue={node?.name} required maxLength={100} /></label>
            <label>环境<select name="kind" defaultValue={node?.kind ?? 'linux'}>
              <option value="linux">Linux</option>
              <option value="wsl">WSL</option>
            </select></label>
            <label>宿主机标识<input name="host_name" defaultValue={node?.host_name ?? undefined} maxLength={100} /></label>
            <label>标签<textarea name="tags" defaultValue={node?.tags.join('\n')} placeholder="每行一个标签" rows={3} /></label>
            <label>备注<textarea name="notes" defaultValue={node?.notes} rows={3} maxLength={1000} /></label>
            {error && <p className="error" role="alert">{error}</p>}
            <div className="dialog-actions">
              {!node && <button type="button" onClick={() => goTo('form')} disabled={busy}>SSH 接入</button>}
              <button type="button" onClick={onClose} disabled={busy}>取消</button>
              <button className="primary" type="submit" disabled={busy}>{node ? '保存' : '生成注册凭证'}</button>
            </div>
          </form>
        )}
        {step === 'form' && node && !deploy && (
          <div className="credential-actions">
            <a className="text-link" href={'/nodes/' + node.id + '/deploy'}>部署 Agent</a>
            <button className="danger quiet" type="button" disabled={busy} onClick={() => goTo('revoke')}>吊销凭证</button>
            <button className="danger quiet" type="button" disabled={busy} onClick={() => goTo('remove')}>移除节点</button>
          </div>
        )}
        {(step === 'rotate' || step === 'revoke' || step === 'remove') && (
          <>
            {step === 'remove' ? (
              <p>将移除节点 <strong>{node?.name}</strong>，并吊销该节点的全部凭证。</p>
            ) : (
              <p>节点 <strong>{node?.name}</strong> 的现有凭证将失效。{step === 'rotate' && '生成新凭证后，使用下方配置手动接入 Agent。已注册的 Agent 需要先停止并备份旧身份文件。'}</p>
            )}
            {error && <p className="error" role="alert">{error}</p>}
            <div className="dialog-actions">
              <button type="button" onClick={() => goTo('form')} disabled={busy}>返回</button>
              <button className={step === 'rotate' ? 'primary' : 'danger primary'} type="button" onClick={confirm} disabled={busy}>
                {step === 'remove' ? '确认移除' : step === 'revoke' ? '确认吊销' : '确认更新'}
              </button>
            </div>
          </>
        )}
        {step === 'token' && enrollment && <AgentEnrollment enrollment={enrollment} reEnroll={Boolean(node)} onClose={onClose} />}
      </section>}
    </>
  );
}
