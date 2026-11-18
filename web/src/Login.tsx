import { useState, type FormEvent } from 'react';
import { errorMessage, request, type Session } from './api';

export function Login({ onSignIn }: { onSignIn: (session: Session) => void }) {
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const form = event.currentTarget;
    const fields = new FormData(form);
    setBusy(true);
    setError('');
    try {
      const session = await request<Session>('/auth/login', {
        method: 'POST',
        body: { username: fields.get('username'), password: fields.get('password') },
      });
      form.reset();
      onSignIn(session);
    } catch (failure) {
      setError(errorMessage(failure));
    } finally {
      const password = form.elements.namedItem('password');
      if (password instanceof HTMLInputElement) password.value = '';
      setBusy(false);
    }
  }

  return (
    <main className="shell login">
      <h1>登录</h1>
      <form className="fields" method="post" onSubmit={submit}>
        <label>
          用户名
          <input name="username" autoComplete="username" required maxLength={64} />
        </label>
        <label>
          密码
          <input name="password" type="password" autoComplete="current-password" required />
        </label>
        {error && <p className="error" role="alert">{error}</p>}
        <button className="primary" type="submit" disabled={busy}>{busy ? '正在登录…' : '登录'}</button>
      </form>
    </main>
  );
}
