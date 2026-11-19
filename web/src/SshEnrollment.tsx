import { useEffect, useRef, useState, type FormEvent } from 'react';
import { ApiError, errorMessage, isUnauthorized, request, type Node, type NodeDraft } from './api';
import { Problem } from './ui';
import { DeploymentLog } from './DeploymentLog';
import { useResource } from './useResource';

type Inspection = {
  ticket: string; address: string; fingerprint: string; center: string; commands: string[];
};
type Job = { id: string; node_id: string; sshTarget?: string };
type Snapshot = { state: 'running' | 'failed' | 'deployed'; heartbeat: boolean; lines: string[] };
type Props = { existing?: Node; onManual: () => void; onExpired: () => void };

function previous(key: string, nodeId?: string): Job | undefined {
  try {
    const value = JSON.parse(sessionStorage.getItem(key) ?? 'null') as Job | null;
    return value && (!nodeId || nodeId === value.node_id) && /^[a-f0-9]{32}$/u.test(value.id) && /^[a-f0-9]{32}$/u.test(value.node_id) ? value : undefined;
  } catch { return undefined; }
}

export function SshEnrollment({ existing, onManual, onExpired }: Props) {
  const lastJob = 'asterion.ssh-enrollment.job' + (existing ? '.' + existing.id : '');
  const capability = useResource<{ enabled: boolean }>('/nodes/ssh/capability', onExpired);
  const [node, setNode] = useState<NodeDraft>({ name: '', kind: 'linux', tags: [], notes: '' });
  const [target, setTarget] = useState({ host: '', port: '22', user: '' });
  const [plan, setPlan] = useState<Inspection>();
  const [method, setMethod] = useState<'password' | 'key'>('password');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [job, setJob] = useState<Job | undefined>(() => previous(lastJob, existing?.id));
  const [snapshot, setSnapshot] = useState<Snapshot>();
  const [retry, setRetry] = useState(0);
  const active = useRef(true);
  useEffect(() => { active.current = true; return () => { active.current = false; }; }, []);
  useEffect(() => {
    if (!job) return;
    const controller = new AbortController();
    let timer: number | undefined;
    async function poll(id: string) {
      try {
        const update = await request<Snapshot>('/nodes/ssh/deploy/' + id, { signal: controller.signal });
        if (controller.signal.aborted) return;
        setSnapshot(update);
        setError('');
        if (update.state === 'failed' || (update.state === 'deployed' && update.heartbeat)) return;
        timer = window.setTimeout(() => void poll(id), update.state === 'running' ? 650 : 3000);
      } catch (failure) {
        if (controller.signal.aborted) return;
        if (isUnauthorized(failure)) onExpired();
        else if (failure instanceof ApiError && failure.status === 404) {
          sessionStorage.removeItem(lastJob);
          setJob(undefined);
          setError('部署日志已失效，可在节点列表查看接入状态。');
        } else {
          setError('日志连接中断；重新读取不会重复部署。');
        }
      }
    }
    void poll(job.id);
    return () => { controller.abort(); window.clearTimeout(timer); };
  }, [job, retry, onExpired, lastJob]);

  async function inspect(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (busy) return;
    setBusy(true);
    setError('');
    try {
      const answer = await request<Inspection | { error: string }>('/nodes/ssh/inspect', {
        method: 'POST', body: { host: target.host.trim(), port: Number(target.port), user: target.user.trim(), node_id: existing?.id },
      });
      if (!active.current) return;
      if ('error' in answer) setError(answer.error);
      else setPlan(answer);
    } catch (failure) {
      if (isUnauthorized(failure)) onExpired(); else setError(errorMessage(failure));
    } finally { if (active.current) setBusy(false); }
  }

  async function deploy(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!plan || busy) return;
    const form = event.currentTarget;
    const values = new FormData(form);
    setBusy(true);
    setError('');
    try {
      const started = await request<Job>('/nodes/ssh/deploy', { method: 'POST', body: {
        ticket: plan.ticket, fingerprint: plan.fingerprint, node: existing ? undefined : { ...node, tags: node.tags.map((tag) => tag.trim()).filter(Boolean) }, method,
        secret: values.get('secret'), passphrase: method === 'key' ? values.get('passphrase') || null : null,
      } });
      const deployedJob = { ...started, sshTarget: target.user.trim() + '@' + target.host.trim() };
      try { sessionStorage.setItem(lastJob, JSON.stringify(deployedJob)); } catch { setError('当前浏览器不支持重新打开部署日志。'); }
      if (active.current) {
        form.reset();
        setPlan(undefined);
        setJob(deployedJob);
      }
    } catch (failure) {
      if (isUnauthorized(failure)) onExpired(); else setError(errorMessage(failure));
    } finally { if (active.current) setBusy(false); }
  }

  if (job) return <div className="fields">
    <h3>{!snapshot ? '读取部署日志' : snapshot.state === 'running' ? '正在部署 Agent'
      : snapshot.state === 'failed' ? '部署失败' : '部署完成，SSH 已断开'}</h3>
    {snapshot && <p role="status">{snapshot.heartbeat ? '已收到 Agent 心跳' : '尚未收到 Agent 心跳'}</p>}
    {error && <Problem error={error} retry={() => setRetry((value) => value + 1)} />}
    <div className="dialog-actions">
      <a className="text-link" href={'/nodes/' + job.node_id}>查看节点</a>
      {snapshot && snapshot.state !== 'running' && <button type="button" onClick={() => {
        sessionStorage.removeItem(lastJob); setJob(undefined); setSnapshot(undefined); setError('');
      }}>{existing ? '重新部署 Agent' : '接入另一台服务器'}</button>}
    </div>
    <h2>执行命令与日志</h2>
    <DeploymentLog lines={snapshot?.lines ?? []} target={job.sshTarget} />
  </div>;
  if (plan) return <form className="fields" onSubmit={deploy}>
    <dl className="detail-facts">
      <div><dt>部署目标</dt><dd>{target.user}@{target.host}:{target.port}（{plan.address}）</dd></div>
      <div><dt>SSH 主机指纹</dt><dd className="technical">{plan.fingerprint}</dd></div>
      <div><dt>Agent 回连中心</dt><dd>{plan.center}</dd></div>
    </dl>
    <label className="checkbox-field"><input type="checkbox" required /> 已核对主机指纹，允许部署并启动 Agent</label>
    <label>SSH 认证<select value={method} onChange={(event) => setMethod(event.target.value === 'key' ? 'key' : 'password')}>
      <option value="password">密码</option><option value="key">私钥</option></select></label>
    {method === 'password' ? <label>SSH 密码<input name="secret" type="password" autoComplete="off" maxLength={1024} required /></label>
      : <><label>SSH 私钥<textarea name="secret" rows={4} spellCheck={false} autoComplete="off" maxLength={32768} required /></label>
        <label>私钥解密口令（可选）<input name="passphrase" type="password" autoComplete="off" maxLength={1024} /></label></>}
    {error && <p className="error" role="alert">{error}</p>}
    <div className="dialog-actions"><button type="button" disabled={busy} onClick={() => { setError(''); setPlan(undefined); }}>返回</button>
      <button className="primary" disabled={busy}>{busy ? '正在提交…' : existing && existing.status !== 'pending' ? '重新部署并启动' : '一键部署并启动'}</button></div>
    <h2>将执行的命令</h2>
    <pre className="output deployment-output" tabIndex={0}>{plan.commands.join('\n\n')}</pre>
  </form>;
  return <form className="fields" onSubmit={inspect}>
    <div className="form-grid">
      {!existing && <><label>节点名称<input required maxLength={100} value={node.name} onChange={(event) => setNode({ ...node, name: event.target.value })} /></label>
        <label>环境<select value={node.kind} onChange={(event) => setNode({ ...node, kind: event.target.value === 'wsl' ? 'wsl' : 'linux' })}>
          <option value="linux">Linux</option><option value="wsl">WSL</option></select></label></>}
      <label>SSH 主机<input required maxLength={253} spellCheck={false} value={target.host} onChange={(event) => setTarget({ ...target, host: event.target.value })} /></label>
      <label>SSH 端口<input required type="number" min={1} max={65535} value={target.port} onChange={(event) => setTarget({ ...target, port: event.target.value })} /></label>
      <label>SSH 用户<input required maxLength={64} value={target.user} onChange={(event) => setTarget({ ...target, user: event.target.value })} /></label>
    </div>
    {!existing && <details><summary>节点信息（可选）</summary><div className="fields">
      <label>宿主机标识<input maxLength={100} value={node.host_name ?? ''} onChange={(event) => setNode({ ...node, host_name: event.target.value || null })} /></label>
      <label>标签<textarea rows={2} placeholder="每行一个标签" value={node.tags.join('\n')}
        onChange={(event) => setNode({ ...node, tags: event.target.value.split('\n') })} /></label>
      <label>备注<textarea rows={2} maxLength={1000} value={node.notes} onChange={(event) => setNode({ ...node, notes: event.target.value })} /></label>
    </div></details>}
    {capability.error && <Problem error={capability.error} retry={capability.refresh} />}
    {capability.data && !capability.data.enabled && <p>中心未配置 SSH 部署，可使用手动接入。</p>}
    {error && <p role="alert" className="error">{error}</p>}
    <div className="dialog-actions">
      <button type="button" disabled={busy} onClick={onManual}>手动接入</button>
      <button className="primary" disabled={busy || !capability.data?.enabled}>{busy ? '正在读取指纹…' : '核对主机与部署命令'}</button></div>
  </form>;
}
