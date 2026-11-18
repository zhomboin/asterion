import { useEffect, useRef, useState, type FormEvent } from 'react';
import {
  errorMessage, formatTime, isUnauthorized, request,
  type Enrollment, type Node, type NodeDraft,
} from './api';

type Props = {
  node?: Node;
  onClose: () => void;
  onUpdated: () => void;
  onExpired: () => void;
};

type Step = 'form' | 'rotate' | 'revoke' | 'remove' | 'token';

export function NodeDialog({ node, onClose, onUpdated, onExpired }: Props) {
  const dialog = useRef<HTMLDialogElement>(null);
  const [step, setStep] = useState<Step>('form');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const [enrollment, setEnrollment] = useState<Enrollment>();
  const [copied, setCopied] = useState(false);

  useEffect(() => {
    const element = dialog.current;
    if (element && !element.open) element.showModal();
    return () => element?.close();
  }, []);

  async function perform(action: () => Promise<void>) {
    setBusy(true);
    setError('');
    try {
      await action();
      onUpdated();
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

  async function copyToken() {
    if (!enrollment) return;
    try {
      await navigator.clipboard.writeText(enrollment.token);
      setCopied(true);
      setError('');
    } catch {
      setError('无法复制，请手动选中注册凭证。');
    }
  }

  function goTo(next: Step) {
    setError('');
    setStep(next);
  }

  const titles = {
    form: node ? '管理节点' : '登记节点',
    rotate: '更新注册凭证',
    revoke: '吊销凭证',
    remove: '移除节点',
    token: '注册凭证',
  };

  return (
    <dialog ref={dialog} aria-labelledby="node-dialog-title" onCancel={(event) => {
      event.preventDefault();
      if (!busy) onClose();
    }}>
      <h2 id="node-dialog-title">{titles[step]}</h2>
      {step === 'form' && (
        <form className="fields" onSubmit={save}>
          <label>节点名称<input name="name" defaultValue={node?.name} required maxLength={100} /></label>
          <label>环境<select name="kind" defaultValue={node?.kind ?? 'linux'}>
            <option value="linux">Linux</option>
            <option value="wsl">WSL</option>
          </select></label>
          <label>标签<textarea name="tags" defaultValue={node?.tags.join('\n')} placeholder="每行一个标签" rows={3} /></label>
          <label>备注<textarea name="notes" defaultValue={node?.notes} rows={3} maxLength={1000} /></label>
          {error && <p className="error" role="alert">{error}</p>}
          <div className="dialog-actions">
            <button type="button" onClick={onClose} disabled={busy}>取消</button>
            <button className="primary" type="submit" disabled={busy}>{node ? '保存' : '生成注册凭证'}</button>
          </div>
        </form>
      )}
      {step === 'form' && node && (
        <div className="credential-actions">
          <button className="quiet" type="button" disabled={busy} onClick={() => goTo('rotate')}>更新注册凭证</button>
          <button className="danger quiet" type="button" disabled={busy} onClick={() => goTo('revoke')}>吊销凭证</button>
          <button className="danger quiet" type="button" disabled={busy} onClick={() => goTo('remove')}>移除节点</button>
        </div>
      )}
      {(step === 'rotate' || step === 'revoke' || step === 'remove') && (
        <>
          {step === 'remove' ? (
            <p>将移除节点 <strong>{node?.name}</strong>，并吊销该节点的全部凭证。</p>
          ) : (
            <p>节点 <strong>{node?.name}</strong> 的现有凭证将失效，Agent 需使用新凭证接入。</p>
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
      {step === 'token' && enrollment && (
        <div className="fields">
          <p>仅显示一次，{formatTime(enrollment.expires_at)} 到期。</p>
          <label>一次性注册凭证<textarea rows={3} readOnly spellCheck={false} value={enrollment.token} /></label>
          {error && <p className="error" role="alert">{error}</p>}
          <div className="dialog-actions">
            <button type="button" onClick={copyToken}>{copied ? '已复制' : '复制'}</button>
            <button className="primary" type="button" onClick={onClose}>完成</button>
          </div>
        </div>
      )}
    </dialog>
  );
}
