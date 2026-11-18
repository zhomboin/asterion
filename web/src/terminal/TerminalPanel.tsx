import { useEffect, useRef, useState, type FormEvent } from 'react';
import { errorMessage, isUnauthorized, request } from '../api';
import { TerminalSession } from './TerminalSession';
import { initialTerminalSize } from './Screen';


export function TerminalPanel({ nodeId, onExpired }: { nodeId: string; onExpired: () => void }) {
  const [id, setId] = useState<string>();
  const [error, setError] = useState('');
  const [opening, setOpening] = useState(false);
  const mounted = useRef(false);

  useEffect(() => {
    mounted.current = true;
    return () => { mounted.current = false; };
  }, []);

  async function open(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (opening) return;
    const form = event.currentTarget;
    const password = new FormData(form).get('password');
    setOpening(true);
    setError('');
    try {
      const created = await request<{ id: string }>('/nodes/' + encodeURIComponent(nodeId) + '/terminal', {
        method: 'POST',
        body: { password, ...initialTerminalSize },
      });
      if (!mounted.current) {
        void request('/terminal/' + encodeURIComponent(created.id), {
          method: 'DELETE', keepalive: true, response: 'empty',
        }).catch(() => undefined);
        return;
      }
      setId(created.id);
    } catch (failure) {
      if (!mounted.current) return;
      if (isUnauthorized(failure)) onExpired();
      else setError(errorMessage(failure));
    } finally {
      form.reset();
      setOpening(false);
    }
  }

  return (
    <section className="monitor-view" aria-label="终端">
      {!id && (
        <form className="fields detail-panel terminal-login" method="post" onSubmit={open}>
          <h2>打开终端</h2>
          <label>管理员口令<input name="password" type="password" autoComplete="current-password" required /></label>
          {error && <p role="alert" className="error">{error}</p>}
          <button type="submit" className="primary" disabled={opening}>{opening ? '正在连接…' : '连接'}</button>
        </form>
      )}
      {id && <TerminalSession id={id} onExpired={onExpired} onClose={() => setId(undefined)} />}
    </section>
  );
}
