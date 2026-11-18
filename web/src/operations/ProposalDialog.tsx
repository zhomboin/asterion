import { useEffect, useRef, useState, type FormEvent } from 'react';
import { errorMessage, isUnauthorized, request } from '../api';
import { Timestamp } from '../ui';
import { operationName, operationTarget, type AccountChange, type OperationAction, type PreparedOperation } from './data';

export function ProposalDialog({ nodeId, nodeName, action, resourceName, onClose, onExpired }: {
  nodeId: string;
  nodeName: string;
  action: OperationAction;
  resourceName?: string;
  onClose: () => void;
  onExpired: () => void;
}) {
  const dialog = useRef<HTMLDialogElement>(null);
  const [prepared, setPrepared] = useState<PreparedOperation>();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [now, setNow] = useState(() => Math.floor(Date.now() / 1_000));
  const expired = prepared !== undefined && prepared.expires_at <= now;
  const selected = prepared?.action ?? action;

  useEffect(() => {
    const target = dialog.current;
    if (target && !target.open) target.showModal();
    return () => target?.close();
  }, []);

  useEffect(() => {
    if (!prepared) return;
    const interval = window.setInterval(() => setNow(Math.floor(Date.now() / 1_000)), 1_000);
    return () => window.clearInterval(interval);
  }, [prepared]);

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (busy || expired) return;
    const form = event.currentTarget;
    setBusy(true);
    setError('');
    try {
      if (!prepared) {
        setPrepared(await request<PreparedOperation>('/nodes/' + encodeURIComponent(nodeId) + '/operations', {
          method: 'POST',
          body: action,
        }));
        return;
      }
      const password = new FormData(form).get('password');
      const body = prepared.step_up_required
        ? { challenge: prepared.challenge, password }
        : { challenge: prepared.challenge };
      const path = '/operations/' + encodeURIComponent(prepared.task_id);
      await request(path + '/confirm', { method: 'POST', body, response: 'empty' });
      window.location.assign(path);
    } catch (failure) {
      if (isUnauthorized(failure)) onExpired();
      else setError(errorMessage(failure));
    } finally {
      if (prepared?.step_up_required) form.reset();
      setBusy(false);
    }
  }

  return (
    <dialog className="operation-dialog" ref={dialog} aria-labelledby="operation-dialog-title" onCancel={(event) => {
      event.preventDefault();
      if (!busy) onClose();
    }}>
      <h2 id="operation-dialog-title">{operationName(selected)}</h2>
      <dl className="detail-facts asset-facts">
        <div><dt>节点</dt><dd>{nodeName}</dd></div>
        <div><dt>目标</dt><dd className="technical">{resourceName ?? operationTarget(selected)}</dd></div>
        {selected.type === 'docker' && (
          <>
            <div><dt>Engine ID</dt><dd className="technical">{selected.engine_id}</dd></div>
            <div><dt>容器 ID</dt><dd className="technical">{selected.container_id}</dd></div>
          </>
        )}
        {selected.type === 'process' && (
          <div><dt>进程</dt><dd className="technical">PID {selected.pid} / {selected.start_ticks}</dd></div>
        )}
        {prepared && <div><dt>确认有效至</dt><dd><Timestamp value={prepared.expires_at} /></dd></div>}
        {selected.type === 'account' && <AccountImpact change={selected.change} />}
        {selected.type === 'command' && Object.entries(selected.parameters).map(([key, value]) => (
          <div key={key}><dt>{key}</dt><dd className="technical">{value}</dd></div>
        ))}
      </dl>
      <form className="fields" method="post" onSubmit={submit}>
        {selected.type === 'process' && selected.signal === 'kill' && (
          <p className="error">SIGKILL 会立即终止进程。进程没有机会清理资源。</p>
        )}
        {prepared?.step_up_required && <label>管理员口令<input type="password" name="password" required autoComplete="current-password" /></label>}
        {expired && <p className="error" role="alert">确认已过期，请重新选择操作。</p>}
        {error && (
          <p className="error" role="alert">
            {error}
            {prepared && <> <a className="text-link" href={'/operations/' + encodeURIComponent(prepared.task_id)}>查看任务状态</a></>}
          </p>
        )}
        <div className="dialog-actions">
          <button type="button" onClick={onClose} disabled={busy}>关闭</button>
          <button type="submit" disabled={busy || expired} className="primary">{prepared ? '确认执行' : '准备操作'}</button>
        </div>
      </form>
    </dialog>
  );
}

function AccountImpact({ change }: { change: AccountChange }) {
  if (change.type === 'set_groups') return <div><dt>目标附加组</dt><dd>{change.groups.join('、') || '无'}</dd></div>;
  if (change.type === 'remove_key') return <div><dt>公钥指纹</dt><dd className="technical">{change.fingerprint}</dd></div>;
  if (change.type !== 'add_key') return null;
  const [algorithm, key = ''] = change.key.split(/\s+/u);
  return <div><dt>公钥</dt><dd className="technical">{algorithm} {key.slice(0, 12)}…{key.slice(-8)}</dd></div>;
}
