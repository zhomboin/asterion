const plot = { left: 47, right: 625, top: 13, bottom: 117 };

type PlotPoint = { at: number; values: (number | null)[] };
type Track = { label: string; name: string };

function segments(samples: PlotPoint[], trackIndex: number, maximumGap: number, bucket?: number) {
  const groups: { at: number; value: number }[][] = [];
  let group: { at: number; value: number }[] = [];
  let lastAt: number | undefined;
  for (const { at, values } of samples) {
    const value = values[trackIndex];
    const present = value != null && Number.isFinite(value);
    const gap = lastAt !== undefined && (at - lastAt > maximumGap
      || (bucket !== undefined && Math.floor(at / bucket) - Math.floor(lastAt / bucket) > 1));
    if (!present || gap) {
      if (group.length) groups.push(group);
      group = [];
    }
    if (present) group.push({ at, value });
    lastAt = at;
  }
  if (group.length) groups.push(group);
  return groups;
}

export function TimeSeries({ title, samples, tracks, duration, oldestLabel, maximumGap, bucket, maxValue = 100, tickLabel }: {
  title: string;
  samples: PlotPoint[];
  tracks: Track[];
  duration: number;
  oldestLabel: string;
  maximumGap: number;
  bucket?: number;
  maxValue?: number;
  tickLabel: (value: number) => string;
}) {
  const last = samples.at(-1);
  if (!last) return null;

  const end = Math.max(Math.floor(Date.now() / 1_000), last.at);
  const start = end - duration;
  const x = (at: number) => plot.left
    + (plot.right - plot.left) * Math.max(0, Math.min(1, (at - start) / duration));
  const y = (value: number) => plot.bottom
    - (plot.bottom - plot.top) * Math.max(0, Math.min(1, value / maxValue));
  const latest = tracks.map((track, index) => track.label + ' ' + (last.values[index] == null
    ? '缺测'
    : tickLabel(last.values[index]))).join('，');

  return (
    <figure className="trend">
      <figcaption className="trend-caption">
        <h2>{title}</h2>
        <ul aria-label="图例">
          {tracks.map(({ label, name }) => (
            <li key={name}><span className={'trend-key ' + name} aria-hidden="true" />{label}</li>
          ))}
        </ul>
      </figcaption>
      <svg viewBox="0 0 637 145" role="img" aria-label={title + '，最近采样 ' + latest}>
        {[0, 0.25, 0.5, 0.75, 1].map((ratio) => {
          const tick = ratio * maxValue;
          return (
            <g key={ratio}>
              <line className="trend-grid" x1={plot.left} x2={plot.right} y1={y(tick)} y2={y(tick)} />
              <text className="trend-axis" x="42" y={y(tick) + 3} textAnchor="end">{tickLabel(tick)}</text>
            </g>
          );
        })}
        {tracks.map(({ name }, index) => (
          <g key={name} className={'trend-track ' + name}>
            {segments(samples, index, maximumGap, bucket).map((points) => (
              points.length === 1
                ? <circle key={points[0].at} cx={x(points[0].at)} cy={y(points[0].value)} r="2.5" />
                : <polyline key={points[0].at} points={points.map((point) => x(point.at) + ',' + y(point.value)).join(' ')} />
            ))}
          </g>
        ))}
        <text className="trend-axis" x={plot.left} y="139">{oldestLabel}</text>
        <text className="trend-axis" x={plot.right} y="139" textAnchor="end">现在</text>
      </svg>
    </figure>
  );
}
