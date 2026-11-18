import { useResource } from '../useResource';
import { alertMetric, type Alerts } from './data';

export function OverviewAlerts({ onExpired }: { onExpired: () => void }) {
  const events = useResource<Alerts>('/alerts?state=active&limit=5', onExpired, 10_000);
  if (events.error || !events.data?.alerts.length) return null;
  return (
    <section className="active-alerts" aria-label="进行中告警">
      <div className="section-bar">
        <h2>进行中告警</h2>
        <a href="/alerts" className="text-link">查看告警</a>
      </div>
      <ul>
        {events.data.alerts.map((alert) => (
          <li key={alert.id}>
            <a href={'/nodes/' + encodeURIComponent(alert.node_id)} className="text-link">{alert.node_name}</a>
            <span>{alertMetric[alert.metric]}</span>
          </li>
        ))}
      </ul>
    </section>
  );
}
