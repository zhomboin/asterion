import { useEffect, useRef, useState } from 'react';
import { request } from './api';

type UnknownKey = { status: 'unknown'; algorithm: string; fingerprint: string };
type KeyResult = UnknownKey | { status: 'trusted' };
type Prompt = UnknownKey & { target: string };

export function useHostKeyConfirmation() {
  const [prompt, setPrompt] = useState<Prompt | null>(null);
  const answer = useRef<((accepted: boolean) => void) | null>(null);
  useEffect(() => () => { answer.current?.(false); }, []);

  function finish(accepted: boolean) {
    answer.current?.(accepted);
    answer.current = null;
    setPrompt(null);
  }

  async function verify(target: string) {
    const result = await request<KeyResult>('/ssh/host-key', { method: 'POST', body: { ssh_profile: target } });
    if (result.status === 'trusted') return true;
    const accepted = await new Promise<boolean>(resolve => {
      answer.current = resolve;
      setPrompt({ ...result, target });
    });
    if (!accepted) return false;
    const checked = await request<KeyResult>('/ssh/host-key', {
      method: 'POST', timeoutMs: 18_000, body: { ssh_profile: target, fingerprint: result.fingerprint },
    });
    return checked.status === 'trusted';
  }

  return { verify, dialog: prompt && <HostKeyDialog prompt={prompt} finish={finish} /> };
}

function HostKeyDialog({ prompt, finish }: { prompt: Prompt; finish: (accepted: boolean) => void }) {
  const ref = useRef<HTMLDialogElement>(null);
  useEffect(() => {
    const dialog = ref.current;
    dialog?.showModal();
    return () => dialog?.close();
  }, []);
  return <dialog ref={ref} aria-labelledby="host-key-title" onCancel={event => { event.preventDefault(); finish(false); }}>
    <h2 id="host-key-title">核对服务器指纹</h2>
    <p>首次连接 <strong>{prompt.target.replace(/^ssh:\/\//u, '')}</strong>。请与服务器控制台或管理员提供的 {prompt.algorithm} 指纹核对。</p>
    <p className="output">{prompt.fingerprint}</p>
    <div className="dialog-actions">
      <button type="button" autoFocus onClick={() => finish(false)}>取消</button>
      <button type="button" className="primary" onClick={() => finish(true)}>信任并继续</button>
    </div>
  </dialog>;
}
