import { useCallback, useEffect, useRef, useState } from 'react';
import { errorMessage, isUnauthorized, request, type Session } from './api';
import { AssetDetailPage } from './assets/AssetDetailPage';
import { AssetListPage } from './assets/AssetListPage';
import { Login } from './Login';
import { NodeDetailPage } from './monitor/NodeDetailPage';
import { NodesPage } from './NodesPage';
import { NodeConfigurationPage } from './NodeConfigurationPage';
import { OperationDetailPage } from './operations/OperationDetailPage';
import { OperationsPage } from './operations/OperationsPage';
import { AlertsPage } from './alerts/AlertsPage';
import { SettingsPage } from './SettingsPage';
import { Avatar, Brand, Icon, IconButton, type IconName } from './ui';

type Auth =
  | { state: 'loading' }
  | { state: 'anonymous' }
  | { state: 'authenticated'; session: Session; sessionVerified: boolean }
  | { state: 'failed'; message: string };

type Destination =
  | { page: 'overview' | 'nodes' | 'assets' | 'operations' | 'alerts' | 'settings' }
  | { page: 'node-config'; id?: string; deploy: boolean }
  | { page: 'node' | 'asset' | 'operation'; id: string };

function destination(path: string): Destination | null {
  if (path === '/overview') return { page: 'overview' };
  if (path === '/nodes') return { page: 'nodes' };
  if (path === '/nodes/new') return { page: 'node-config', deploy: true };
  const config = /^\/nodes\/([a-f0-9]{32})\/(manage|deploy)$/u.exec(path);
  if (config) return { page: 'node-config', id: config[1], deploy: config[2] === 'deploy' };
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
        setAuth({ state: 'authenticated', session, sessionVerified: true });
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

    function reportSessionCheck(sessionVerified: boolean) {
      if (controller.signal.aborted) return;
      setAuth((current) => current.state === 'authenticated' && current.sessionVerified !== sessionVerified
        ? { ...current, sessionVerified }
        : current);
    }

    async function checkSession() {
      if (checking || document.hidden) return;
      checking = true;
      try {
        await request<Session>('/me', { signal: controller.signal });
        reportSessionCheck(true);
      } catch (failure) {
        if (controller.signal.aborted) return;
        if (isUnauthorized(failure)) expire();
        else reportSessionCheck(false);
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
    setAuth({ state: 'authenticated', session, sessionVerified: true });
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

  if (auth.state === 'anonymous') return <Login onSignIn={signedIn} />;
  if (auth.state !== 'authenticated') {
    return (
      <main className="stage">
        <div className="stage-card">
          <Brand />
          {auth.state === 'loading' && <p role="status" className="muted">正在连接中心…</p>}
          {auth.state === 'failed' && (
            <>
              <p role="alert">{auth.message}</p>
              <button type="button" className="primary" onClick={() => {
                setAuth({ state: 'loading' });
                setAttempt((count) => count + 1);
              }}>重试</button>
            </>
          )}
        </div>
      </main>
    );
  }

  const username = auth.session.username;
  return (
    <div className="app-frame">
      <aside className="sidebar">
        <Brand />
        <nav className="side-nav" aria-label="主导航">
          {navigation.map(({ href, label, icon, pages }) => (
            <a key={href} href={href} title={label} aria-current={pages.includes(active.page) ? 'page' : undefined}>
              <Icon name={icon} />
              <span className="nav-label">{label}</span>
            </a>
          ))}
        </nav>
        <div className="user-card">
          <Avatar name={username} size="lg" />
          <div className="user-meta">
            <strong>{username}</strong>
            <span>管理员</span>
          </div>
          <IconButton icon="logout" label="退出登录" disabled={loggingOut} onClick={logout} />
        </div>
      </aside>
      <div className="workspace">
        <header className="topbar">
          <span className="top-pill">
            <span className="top-pill-icon"><Icon name="calendar" size={16} /></span>
            <span>{today.format(new Date())}</span>
          </span>
          <span className="top-pill">
            <span className="top-pill-icon" data-tone="light" data-status={auth.sessionVerified ? 'online' : 'pending'}>
              <Icon name="pulse" size={16} />
            </span>
            <span className="top-pill-text" role="status">
              <strong>{auth.sessionVerified ? '中心在线' : '会话校验失败'}</strong>
              <small>每 30 秒校验会话</small>
            </span>
          </span>
          <span className="topbar-spacer" />
          <a className="icon-button" data-tone="inverse" href="/alerts" aria-label="查看告警" title="查看告警">
            <Icon name="alerts" />
          </a>
        </header>
        <main className="dashboard">
          {logoutError && <p className="notice" role="alert">{logoutError}</p>}
          {(active.page === 'overview' || active.page === 'nodes') && <NodesPage manage={active.page === 'nodes'} onExpired={expire} />}
          {active.page === 'node' && <NodeDetailPage key={active.id} id={active.id} onExpired={expire} />}
          {active.page === 'node-config' && <NodeConfigurationPage id={active.id} deploy={active.deploy} onExpired={expire} />}
          {active.page === 'assets' && <AssetListPage onExpired={expire} />}
          {active.page === 'asset' && <AssetDetailPage id={active.id} onExpired={expire} />}
          {active.page === 'operations' && <OperationsPage onExpired={expire} />}
          {active.page === 'operation' && <OperationDetailPage id={active.id} onExpired={expire} />}
          {active.page === 'alerts' && <AlertsPage onExpired={expire} />}
          {active.page === 'settings' && <SettingsPage username={username} onExpired={expire} />}
        </main>
      </div>
    </div>
  );
}

const today = new Intl.DateTimeFormat('zh-CN', { month: 'long', day: 'numeric', weekday: 'short' });

const navigation: { href: string; label: string; icon: IconName; pages: Destination['page'][] }[] = [
  { href: '/overview', label: '总览', icon: 'overview', pages: ['overview'] },
  { href: '/nodes', label: '节点', icon: 'nodes', pages: ['nodes', 'node', 'node-config'] },
  { href: '/assets', label: '资产', icon: 'assets', pages: ['assets', 'asset'] },
  { href: '/operations', label: '运维任务', icon: 'operations', pages: ['operations', 'operation'] },
  { href: '/alerts', label: '告警', icon: 'alerts', pages: ['alerts'] },
  { href: '/settings', label: '设置', icon: 'settings', pages: ['settings'] },
];
