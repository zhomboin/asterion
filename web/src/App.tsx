import { useCallback, useEffect, useRef, useState } from 'react';
import { errorMessage, isUnauthorized, request, type Session } from './api';
import { AssetDetailPage } from './assets/AssetDetailPage';
import { AssetListPage } from './assets/AssetListPage';
import { Login } from './Login';
import { NodeDetailPage } from './monitor/NodeDetailPage';
import { NodesPage } from './NodesPage';
import { OperationDetailPage } from './operations/OperationDetailPage';
import { OperationsPage } from './operations/OperationsPage';
import { AlertsPage } from './alerts/AlertsPage';
import { SettingsPage } from './SettingsPage';

type Auth =
  | { state: 'loading' }
  | { state: 'anonymous' }
  | { state: 'authenticated'; session: Session }
  | { state: 'failed'; message: string };

type Destination =
  | { page: 'overview' | 'nodes' | 'assets' | 'operations' | 'alerts' | 'settings' }
  | { page: 'node' | 'asset' | 'operation'; id: string };

function destination(path: string): Destination | null {
  if (path === '/overview') return { page: 'overview' };
  if (path === '/nodes') return { page: 'nodes' };
  if (path === '/assets') return { page: 'assets' };
  if (path === '/operations') return { page: 'operations' };
  if (path === '/alerts') return { page: 'alerts' };
  if (path === '/settings') return { page: 'settings' };
  const node = /^\/nodes\/([a-f0-9]{32})$/u.exec(path);
  if (node) return { page: 'node', id: node[1] };
  const asset = /^\/assets\/([a-f0-9]{32})$/u.exec(path);
  if (asset) return { page: 'asset', id: asset[1] };
  const operation = /^\/operations\/([a-f0-9]{32})$/u.exec(path);
  if (operation) return { page: 'operation', id: operation[1] };
  return null;
}

function currentDestination() {
  const { pathname, search } = window.location;
  return destination(pathname) ? pathname + search : '/overview';
}

function replacePath(path: string) {
  if (window.location.pathname + window.location.search !== path) {
    window.history.replaceState(null, '', path);
  }
}

export function App() {
  const [auth, setAuth] = useState<Auth>({ state: 'loading' });
  const [attempt, setAttempt] = useState(0);
  const [loggingOut, setLoggingOut] = useState(false);
  const [logoutError, setLogoutError] = useState('');
  const returnTo = useRef(currentDestination());

  const expire = useCallback(() => {
    if (destination(window.location.pathname)) returnTo.current = currentDestination();
    replacePath('/login');
    setLogoutError('');
    setAuth({ state: 'anonymous' });
  }, []);

  useEffect(() => {
    const controller = new AbortController();
    async function loadSession() {
      try {
        const session = await request<Session>('/me', { signal: controller.signal });
        if (controller.signal.aborted) return;
        replacePath(currentDestination());
        setAuth({ state: 'authenticated', session });
      } catch (error) {
        if (controller.signal.aborted) return;
        if (isUnauthorized(error)) expire();
        else setAuth({ state: 'failed', message: errorMessage(error) });
      }
    }
    void loadSession();
    return () => controller.abort();
  }, [attempt, expire]);

  const authenticated = auth.state === 'authenticated';
  useEffect(() => {
    if (!authenticated) return;
    const controller = new AbortController();
    let checking = false;

    async function checkSession() {
      if (checking || document.hidden) return;
      checking = true;
      try {
        await request<Session>('/me', { signal: controller.signal });
      } catch (failure) {
        if (!controller.signal.aborted && isUnauthorized(failure)) expire();
      } finally {
        checking = false;
      }
    }

    function restore(event: PageTransitionEvent) {
      if (!event.persisted) return;
      setAuth({ state: 'loading' });
      setAttempt((count) => count + 1);
    }

    const timer = window.setInterval(() => void checkSession(), 30_000);
    document.addEventListener('visibilitychange', checkSession);
    window.addEventListener('pageshow', restore);
    return () => {
      controller.abort();
      window.clearInterval(timer);
      document.removeEventListener('visibilitychange', checkSession);
      window.removeEventListener('pageshow', restore);
    };
  }, [authenticated, expire]);

  function signedIn(session: Session) {
    replacePath(returnTo.current);
    setAuth({ state: 'authenticated', session });
  }

  async function logout() {
    setLoggingOut(true);
    setLogoutError('');
    try {
      await request<void>('/auth/logout', { method: 'POST' });
      expire();
      returnTo.current = '/overview';
    } catch (error) {
      if (isUnauthorized(error)) {
        expire();
        returnTo.current = '/overview';
      } else {
        setLogoutError(errorMessage(error));
      }
    } finally {
      setLoggingOut(false);
    }
  }

  const active = destination(window.location.pathname) ?? { page: 'overview' };
  const nodeSection = active.page === 'nodes' || active.page === 'node';
  const assetSection = active.page === 'assets' || active.page === 'asset';
  const operationsSection = active.page === 'operations' || active.page === 'operation';

  return (
    <>
      <header className="site-header">
        <div className="shell topbar">
          <a className="wordmark" href="/overview">Asterion</a>
          {auth.state === 'authenticated' && (
            <>
              <nav aria-label="主导航">
                <a href="/overview" aria-current={active.page === 'overview' ? 'page' : undefined}>总览</a>
                <a href="/nodes" aria-current={nodeSection ? 'page' : undefined}>节点</a>
                <a href="/assets" aria-current={assetSection ? 'page' : undefined}>资产</a>
                <a href="/operations" aria-current={operationsSection ? 'page' : undefined}>运维</a>
                <a href="/alerts" aria-current={active.page === 'alerts' ? 'page' : undefined}>告警</a>
                <a href="/settings" aria-current={active.page === 'settings' ? 'page' : undefined}>设置</a>
              </nav>
              <div className="account">
                <span>{auth.session.username}</span>
                <button className="quiet" type="button" disabled={loggingOut} onClick={logout}>退出</button>
              </div>
            </>
          )}
        </div>
      </header>
      {auth.state === 'loading' && <main className="shell blank"><p role="status">正在连接中心…</p></main>}
      {auth.state === 'failed' && (
        <main className="shell blank">
          <p role="alert">{auth.message}</p>
          <button type="button" onClick={() => {
            setAuth({ state: 'loading' });
            setAttempt((count) => count + 1);
          }}>重试</button>
        </main>
      )}
      {auth.state === 'anonymous' && <Login onSignIn={signedIn} />}
      {auth.state === 'authenticated' && (
        <main className="shell dashboard">
          {logoutError && <p className="error" role="alert">{logoutError}</p>}
          {(active.page === 'overview' || active.page === 'nodes') && <NodesPage manage={active.page === 'nodes'} onExpired={expire} />}
          {active.page === 'node' && <NodeDetailPage id={active.id} onExpired={expire} />}
          {active.page === 'assets' && <AssetListPage onExpired={expire} />}
          {active.page === 'asset' && <AssetDetailPage id={active.id} onExpired={expire} />}
          {active.page === 'operations' && <OperationsPage onExpired={expire} />}
          {active.page === 'operation' && <OperationDetailPage id={active.id} onExpired={expire} />}
          {active.page === 'alerts' && <AlertsPage onExpired={expire} />}
          {active.page === 'settings' && <SettingsPage username={auth.session.username} onExpired={expire} />}
        </main>
      )}
    </>
  );
}
