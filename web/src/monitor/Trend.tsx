import { fraction } from '../ui';
import type { MetricPoint, MetricWindow } from './data';
import { TimeSeries } from './TimeSeries';

const duration = { '1h': 3_600, '24h': 86_400 };
const tracks = [
  { label: 'CPU', name: 'cpu' },
  { label: '内存', name: 'memory' },
];

export function Trend({ samples, range, interval = 15, bucket = range === '24h' ? 240 : 10 }: {
  samples: MetricPoint[];
  range: MetricWindow;
  interval?: number;
  bucket?: number;
}) {
  const points = samples.map(({ observed_at: at, sample }) => ({
    at,
    values: [
      sample.cpu_percent,
      sample.memory.total_bytes > 0 ? 100 * sample.memory.used_bytes / sample.memory.total_bytes : null,
    ],
  }));
  return (
    <TimeSeries
      title="资源趋势"
      samples={points}
      tracks={tracks}
      duration={duration[range]}
      oldestLabel={range === '1h' ? '60 分钟前' : '24 小时前'}
      maximumGap={Math.max(interval, bucket) * 1.8}
      bucket={bucket >= interval ? bucket : undefined}
      tickLabel={fraction}
    />
  );
}
