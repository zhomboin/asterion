import { useState } from 'react';
import { bytes, DataTable, fraction, Problem, Tabs, Timestamp } from '../ui';
import { useResource } from '../useResource';
import type { Container, ContainerLogs, ContainerStats } from './data';
import { ServiceActions } from '../operations/ServiceActions';
import type { NodeOperations } from '../operations/data';

const views = [['info', '元数据'], ['stats', '资源'], ['logs', '日志']] as const;
type View = typeof views[number][0];

export function ContainerDetail({ base, item, engineId, onClose, onExpired, operations }: {
  base: string;
  item: Container;
  engineId: string | null;
  operations?: NodeOperations;
  onClose: () => void;
  onExpired: () => void;
}) {
  const [view, setView] = useState<View>('info');

  return (
    <section className="monitor-section detail-panel" aria-label="容器详情">
      <div className="section-bar">
        <div><h2>{item.name}</h2><p className="technical page-scope">{item.id}</p></div>
        <button type="button" className="quiet" onClick={onClose}>关闭</button>
      </div>
      <Tabs label="容器详情视图" size="sm" items={views} value={view} onChange={setView} />
      {view === 'info' && <Metadata item={item} />}
      {view === 'info' && engineId && operations?.catalog.docker_targets.some((target) => target.engine_id === engineId && target.container_ids.includes(item.id)) && (
        <ServiceActions title="容器运维" onChoose={(verb) => {
          operations.propose({ type: 'docker', engine_id: engineId, container_id: item.id, verb }, item.name);
        }} />
      )}
      {view === 'stats' && <Stats base={base} onExpired={onExpired} />}
      {view === 'logs' && <Logs base={base} onExpired={onExpired} />}
    </section>
  );
}

function Metadata({ item }: { item: Container }) {
  return (
    <>
      <dl className="detail-facts">
        <div><dt>状态</dt><dd>{item.state}</dd></div>
        <div><dt>健康</dt><dd>{item.health ?? '—'}</dd></div>
        <div><dt>启动时间</dt><dd><Timestamp value={item.started_at} /></dd></div>
        <div><dt>重启次数</dt><dd>{item.restart_count ?? '—'}</dd></div>
        <div><dt>Compose 项目</dt><dd>{item.compose_project ?? '未标记'}</dd></div>
        <div><dt>Compose 服务</dt><dd>{item.compose_service ?? '未标记'}</dd></div>
      </dl>
      <section className="monitor-section">
        <h3>挂载</h3>
        {item.mounts.length ? (
          <DataTable label="容器挂载" columns={['来源', '容器路径', '模式']}>
            {item.mounts.map((mount, index) => (
              <tr key={index}>
                <td className="technical">{mount.source || '—'}</td>
                <th scope="row" className="technical">{mount.destination}</th><td>{mount.mode || '—'}</td>
              </tr>
            ))}
          </DataTable>
        ) : <p className="empty">无挂载</p>}
      </section>
      <section className="monitor-section">
        <h3>端口</h3>
        {item.ports.length ? (
          <DataTable label="容器端口" columns={['容器端口', '绑定地址', '主机端口', '协议']}>
            {item.ports.map((port, index) => (
              <tr key={index}>
                <th scope="row">{port.private_port}</th><td>{port.ip ?? '—'}</td>
                <td>{port.public_port ?? '—'}</td><td>{port.protocol}</td>
              </tr>
            ))}
          </DataTable>
        ) : <p className="empty">无端口记录</p>}
      </section>
    </>
  );
}

function Stats({ base, onExpired }: { base: string; onExpired: () => void }) {
  const stats = useResource<ContainerStats>(base + '/stats', onExpired);
  return (
    <>
      <div className="section-bar"><h3>资源快照</h3><button type="button" onClick={stats.refresh}>刷新</button></div>
      {stats.loading && <p role="status">正在读取资源指标…</p>}
      {stats.error && <Problem error={stats.error} retry={stats.refresh} />}
      {stats.data && (
        <dl className="detail-facts">
          <div><dt>CPU</dt><dd>{fraction(stats.data.cpu_percent)}</dd></div>
          <div><dt>已用内存 / 限额</dt><dd>{bytes(stats.data.memory_bytes)} / {bytes(stats.data.memory_limit_bytes)}</dd></div>
          <div><dt>累计接收</dt><dd>{bytes(stats.data.rx_bytes)}</dd></div>
          <div><dt>累计发送</dt><dd>{bytes(stats.data.tx_bytes)}</dd></div>
        </dl>
      )}
    </>
  );
}

function Logs({ base, onExpired }: { base: string; onExpired: () => void }) {
  const [lines, setLines] = useState('100');
  const logs = useResource<ContainerLogs>(base + '/logs?lines=' + lines, onExpired);
  return (
    <>
      <div className="section-bar">
        <h3>最近日志</h3>
        <label className="inline-field">行数<select value={lines} onChange={(event) => setLines(event.target.value)}>
          {[25, 100, 200].map((value) => <option key={value} value={value}>{value}</option>)}
        </select></label>
        <button type="button" onClick={logs.refresh}>刷新</button>
      </div>
      {logs.loading && <p role="status">正在读取日志…</p>}
      {logs.error && <Problem error={logs.error} retry={logs.refresh} />}
      {logs.data && (
        <>
          {logs.data.truncated && <p className="observation">输出已截断</p>}
          {logs.data.text ? <pre className="output" tabIndex={0} aria-label="日志内容">{logs.data.text}</pre> : <p className="empty">暂无日志输出</p>}
          <p className="observation">日志仍可能含敏感内容。</p>
        </>
      )}
    </>
  );
}
