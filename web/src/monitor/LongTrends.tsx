import { fraction, perSecond, Problem, Timestamp } from '../ui';
import { useResource } from '../useResource';
import { TimeSeries } from './TimeSeries';

export type LongWindow = '7d' | '30d';

type Bucket = {
  observed_at: number;
  cpu_percent: number | null;
  memory_percent: number | null;
  disk_percent: number | null;
  rx_bytes_per_second: number | null;
  tx_bytes_per_second: number | null;
  sample_count: number;
};
type Trends = { range: LongWindow; bucket_seconds: number; samples: Bucket[] };

const resources = [
  { label: 'CPU', name: 'cpu' },
  { label: '内存', name: 'memory' },
  { label: '文件系统', name: 'disk' },
];
const network = [
  { label: '接收', name: 'cpu' },
  { label: '发送', name: 'memory' },
];

export function LongTrends({ base, range, onExpired }: { base: string; range: LongWindow; onExpired: () => void }) {
  const result = useResource<Trends>(base + '/trends?range=' + range, onExpired, 60_000);
  const samples = result.data?.samples ?? [];
  const last = samples.at(-1);
  const maximum = samples.reduce((current, item) => Math.max(current, item.rx_bytes_per_second ?? 0, item.tx_bytes_per_second ?? 0), 1);
  const duration = (range === '7d' ? 7 : 30) * 86_400;
  const oldestLabel = range === '7d' ? '7 天前' : '30 天前';

  return (
    <>
      {result.loading && <p role="status">正在读取趋势…</p>}
      {result.error && <Problem error={result.error} retry={result.refresh} />}
      {result.data && !last && <p className="empty">此时间范围暂无采样</p>}
      {result.data && last && (
        <>
          <p className="observation">
            来源 · SSH · 聚合间隔 {result.data.bucket_seconds} 秒 · 最近观测 <Timestamp value={last.observed_at} />
          </p>
          <TimeSeries
            title="资源趋势"
            samples={samples.map((item) => ({ at: item.observed_at, values: [item.cpu_percent, item.memory_percent, item.disk_percent] }))}
            tracks={resources}
            duration={duration}
            oldestLabel={oldestLabel}
            maximumGap={result.data.bucket_seconds * 1.5}
            bucket={result.data.bucket_seconds}
            tickLabel={fraction}
          />
          <TimeSeries
            title="网络速率趋势"
            samples={samples.map((item) => ({ at: item.observed_at, values: [item.rx_bytes_per_second, item.tx_bytes_per_second] }))}
            tracks={network}
            duration={duration}
            oldestLabel={oldestLabel}
            maximumGap={result.data.bucket_seconds * 1.5}
            bucket={result.data.bucket_seconds}
            maxValue={maximum}
            tickLabel={perSecond}
          />
        </>
      )}
    </>
  );
}
