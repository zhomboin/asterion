import { useState } from 'react';
import { bytes, DataTable, fraction, perSecond, Problem, Timestamp } from '../ui';
import { useResource } from '../useResource';
import type { MetricHistory, Metrics, MetricWindow } from './data';
import { Trend } from './Trend';
import { LongTrends, type LongWindow } from './LongTrends';

const ranges = [['1h', '最近 1 小时'], ['24h', '最近 24 小时'], ['7d', '最近 7 天'], ['30d', '最近 30 天']] as const;

export function MetricsPanel({ base, onExpired }: { base: string; onExpired: () => void }) {
  const [range, setRange] = useState<MetricWindow | LongWindow>('1h');
  const shortRange = range === '1h' || range === '24h' ? range : null;
  const history = useResource<MetricHistory>(shortRange ? base + '/metrics?range=' + shortRange : null, onExpired, 15_000);
  const samples = history.data?.samples ?? [];
  const last = samples.at(-1);
  const collector = shortRange ? history.data?.collector : null;
  return (
    <section aria-label="主机指标" className="monitor-view">
      <div className="section-bar">
        <h2>主机指标</h2>
        <label className="inline-field">时间范围
          <select value={range} onChange={(event) => setRange(event.target.value as MetricWindow | LongWindow)}>
            {ranges.map(([value, label]) => <option key={value} value={value}>{label}</option>)}
          </select>
        </label>
      </div>
      {collector && collector.pending > 0 && <p role="status">离线记录同步中，尚余 {collector.pending} 条序号待检查</p>}
      {collector && collector.missed > 0 && <p className="observation">累计未能补回 {collector.missed} 条记录（缓存清理或节点时钟异常）</p>}
      {history.loading && <p role="status">正在加载指标…</p>}
      {history.error && <Problem error={history.error} retry={history.refresh} />}
      {history.data && !last && <p className="empty">此时间范围暂无采样</p>}
      {(range === '7d' || range === '30d') && <LongTrends base={base} range={range} onExpired={onExpired} />}
      {shortRange && last && (
        <>
          <p className="observation">{collector ? '服务器采集器' : 'SSH'} · 最近采样 <Timestamp value={last.sample.captured_at} /></p>
          {history.data?.truncated && <p className="observation">较早的指标采样已截断</p>}
          <MetricFacts sample={last.sample} />
          <Trend samples={samples} range={shortRange} interval={history.data?.interval_seconds} bucket={history.data?.bucket_seconds} />
          <section className="monitor-section" aria-label="文件系统">
            <h2>文件系统</h2>
            {last.sample.filesystems.length ? (
              <DataTable label="文件系统" columns={['挂载点', '类型', '容量', '可用']}>
                {last.sample.filesystems.map((fs) => (
                  <tr key={fs.mountpoint}>
                    <th scope="row" className="technical">{fs.mountpoint}</th>
                    <td>{fs.fs_type}</td><td>{bytes(fs.total_bytes)}</td><td>{bytes(fs.available_bytes)}</td>
                  </tr>
                ))}
              </DataTable>
            ) : <p className="empty">无可读取的文件系统</p>}
          </section>
          <section className="monitor-section" aria-label="网络接口">
            <h2>网络接口</h2>
            {last.sample.interfaces.length ? (
              <DataTable label="网络接口" columns={['接口', '接收速率', '发送速率', '累计接收', '累计发送']}>
                {last.sample.interfaces.map((iface) => (
                  <tr key={iface.name}>
                    <th scope="row" className="technical">{iface.name}</th>
                    <td>{perSecond(iface.rx_bytes_per_second)}</td><td>{perSecond(iface.tx_bytes_per_second)}</td>
                    <td>{bytes(iface.rx_bytes)}</td><td>{bytes(iface.tx_bytes)}</td>
                  </tr>
                ))}
              </DataTable>
            ) : <p className="empty">无可读取的网络接口</p>}
          </section>
        </>
      )}
    </section>
  );
}

function Capacity({ used, total }: { used: number; total: number }) {
  return <>{bytes(used)} / {bytes(total)}{total > 0 && ' · ' + fraction(100 * used / total)}</>;
}

function MetricFacts({ sample }: { sample: Metrics }) {
  return (
    <dl className="detail-facts metric-facts">
      <div><dt>CPU 利用率</dt><dd>{fraction(sample.cpu_percent)}</dd></div>
      <div><dt>逻辑核心数</dt><dd>{sample.logical_cores}</dd></div>
      <div><dt>负载 1m / 5m / 15m</dt><dd>{sample.load.join(' / ')}</dd></div>
      <div><dt>内存已用 / 总量</dt><dd><Capacity used={sample.memory.used_bytes} total={sample.memory.total_bytes} /></dd></div>
      <div><dt>可用内存</dt><dd>{bytes(sample.memory.available_bytes)}</dd></div>
      <div><dt>交换空间已用 / 总量</dt><dd><Capacity used={sample.swap.used_bytes} total={sample.swap.total_bytes} /></dd></div>
    </dl>
  );
}
