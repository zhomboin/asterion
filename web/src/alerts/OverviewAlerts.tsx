import { Card } from '../ui';
import { useResource } from '../useResource';
import { alertLabel, alertNodeHref, type Alerts } from './data';

export function OverviewAlerts({ onExpired }: { onExpired: () => void }) {
  const events = useResource<Alerts>('/alerts?state=active&limit=5', onExpired, 10_000);
  const alerts = events.data?.alerts ?? [];
  return (
    <Card
      tone="inverse"
      className="alerts-card"
      label="进行中告警"
      title="进行中告警"
      actions={<a href="/alerts" className="text-link">查看告警</a>}
    >
      <p className="figure">{events.data ? alerts.length + (events.data.has_more ? '+' : '') : '—'}</p>
      {events.error && <p className="muted">告警状态未更新</p>}
      {!events.error && events.data && alerts.length === 0 && <p className="muted">暂无进行中告警</p>}
      {alerts.length > 0 && (
        <ul className="alert-list">
          {alerts.map((alert) => (
            <li key={alert.id}>
              <span className="dot" aria-hidden="true" />
              <a href={alertNodeHref(alert)} className="text-link">{alert.node_name}</a>
              <span className="muted">
                {alertLabel(alert)}{alert.unit && <> · <span className="technical">{alert.unit}</span></>}
              </span>
            </li>
          ))}
        </ul>
      )}
    </Card>
  );
}
