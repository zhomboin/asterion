import { Card, PageHeader, Problem } from './ui';
import { useResource } from './useResource';

type Settings = {
  sample_interval_secs: number;
  metric_retention_days: number;
  trend_retention_days: number;
  terminal_enabled: boolean;
};

export function SettingsPage({ username, onExpired }: { username: string; onExpired: () => void }) {
  const settings = useResource<Settings>('/settings', onExpired);
  return (
    <>
      <PageHeader title="设置" scope="管理面板账号与采集保留策略" />
      <div className="card-stack">
        <Card title="管理面板">
          <dl className="detail-facts asset-facts">
            <div><dt>当前账号</dt><dd>{username}</dd></div>
            <div><dt>连接凭证</dt><dd><a className="text-link" href="/nodes">管理节点凭证</a></dd></div>
          </dl>
        </Card>
        <Card title="采集与保留" label="采集与保留">
          {settings.loading && <p role="status">正在读取设置…</p>}
          {settings.error && <Problem error={settings.error} retry={settings.refresh} />}
          {settings.data && (
            <dl className="detail-facts">
              <div><dt>采样间隔</dt><dd>{settings.data.sample_interval_secs} 秒</dd></div>
              <div><dt>原始指标保留</dt><dd>{settings.data.metric_retention_days} 天</dd></div>
              <div><dt>聚合趋势保留</dt><dd>{settings.data.trend_retention_days} 天</dd></div>
              <div><dt>中心终端功能</dt><dd>{settings.data.terminal_enabled ? '已启用' : '未启用'}</dd></div>
            </dl>
          )}
        </Card>
      </div>
    </>
  );
}
