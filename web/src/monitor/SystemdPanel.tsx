import { useEffect, useRef, useState } from 'react';
import { request, type Node } from '../api';
import { Problem, Timestamp } from '../ui';
import { useResource } from '../useResource';
import type { Systemd, Unit } from './data';
import { ServiceActions } from '../operations/ServiceActions';
import type { NodeOperations } from '../operations/data';
import { ServiceMonitorDetail, ServiceMonitorList, serviceMonitorRefreshMs } from './ServiceMonitoring';
import { MonitorPicker } from './MonitorPicker';
import type { ServiceMonitor } from './serviceMonitoring';

const statusText = {
  available: '可用',
  unavailable: 'systemd 未启用或不可用',
  permission_denied: 'Agent 无权读取 systemd',
  error: 'systemd 查询失败',
};

const liveUnavailableText = {
  online: undefined,
  pending: 'Agent 尚未连接',
  offline: 'Agent 离线',
  unknown: '节点状态未更新',
};

export function SystemdPanel({ base, agentState, initialUnit, onExpired, operations }: {
  base: string;
  agentState: Node['status'] | 'unknown';
  initialUnit?: string;
  onExpired: () => void;
  operations?: NodeOperations;
}) {
  const [adding, setAdding] = useState(false);
  const [includeDisabled, setIncludeDisabled] = useState(false);
  const [selected, setSelected] = useState(initialUnit);
  const opener = useRef<HTMLButtonElement>(null);
  const detailTitle = useRef<HTMLHeadingElement>(null);
  const monitorListTitle = useRef<HTMLHeadingElement>(null);
  const online = agentState === 'online';
  const path = base + '/systemd/services';
  const detail = useResource<Unit>(online && selected ? path + '/' + encodeURIComponent(selected) : null, onExpired);
  const monitorPath = base + '/service-monitors';
  const monitors = useResource<{ monitors: ServiceMonitor[] }>(monitorPath, onExpired, serviceMonitorRefreshMs);
  const monitor = monitors.data?.monitors.find((service) => service.unit === selected);
  const exactUnit = detail.data?.verified && detail.data.name === selected;
  const ready = Boolean(monitors.data && !monitors.error && (monitor || exactUnit));
  const liveUnavailable = liveUnavailableText[agentState];
  const unavailable = liveUnavailable ?? (monitors.error ? '监控状态未更新' : undefined);

  useEffect(() => {
    if (selected) detailTitle.current?.focus();
  }, [selected]);

  function selectUnit(unit: string, action: HTMLButtonElement) {
    opener.current = action;
    if (selected === unit) detailTitle.current?.focus();
    else setSelected(unit);
  }

  return (
    <section className="monitor-view" aria-label="systemd">
      <section className="monitor-section" aria-labelledby="systemd-service-monitors-title">
        <div className="section-bar">
          <h2 id="systemd-service-monitors-title" tabIndex={-1} ref={monitorListTitle}>已纳管 systemd 服务</h2>
          <div className="row-actions">
            {monitors.data?.monitors.some((item) => !item.enabled) && <label>
              <input type="checkbox" checked={includeDisabled} onChange={(event) => setIncludeDisabled(event.target.checked)} /> 显示已停用
            </label>}
            <button type="button" className="primary" disabled={!online} onClick={(event) => {
              opener.current = event.currentTarget; setAdding(true);
            }}>添加监控</button>
          </div>
        </div>
        {!online && <p className="observation">{liveUnavailable}</p>}
        {monitors.loading && <p role="status">正在读取监控服务…</p>}
        {monitors.error && <Problem error={monitors.error} retry={monitors.refresh} />}
        {monitors.data && <ServiceMonitorList
          monitors={includeDisabled ? monitors.data.monitors : monitors.data.monitors.filter((item) => item.enabled)}
          unavailable={unavailable}
          onSelect={selectUnit}
        />}
      </section>
      {selected && (
        <section className="monitor-section detail-panel" aria-label="服务详情">
          <div className="section-bar">
            <h2 className="technical" tabIndex={-1} ref={detailTitle}>{selected}</h2>
            <button type="button" className="quiet" onClick={() => {
              setSelected(undefined);
              const focusTarget = opener.current?.isConnected ? opener.current : monitorListTitle.current;
              focusTarget?.focus();
            }}>关闭</button>
          </div>
          {detail.loading && <p role="status">正在读取 Unit 属性…</p>}
          {detail.error && <Problem error={detail.error} retry={detail.refresh} />}
          {detail.data && <UnitFacts unit={detail.data} />}
          {(monitor || exactUnit) && <ServiceMonitorDetail
            key={selected}
            path={monitorPath + '/' + encodeURIComponent(selected)}
            monitor={monitor}
            ready={ready}
            unavailable={unavailable}
            onChanged={monitors.refresh}
            onExpired={onExpired}
          />}
          {detail.data && operations?.catalog.systemd_units.includes(selected) && (
            <ServiceActions title="systemd 运维" onChoose={(verb) => {
              operations.propose({ type: 'systemd', unit: selected, verb });
            }} />
          )}
        </section>
      )}
      {liveUnavailable && <p className="empty">{liveUnavailable}，实时查询暂不可用</p>}
      {adding && online && <MonitorPicker<Systemd> title="添加 systemd 服务监控" placeholder="Unit 名称"
        path={(q) => path + (q ? '?' + new URLSearchParams({ q }) : '')}
        results={(data) => data.status !== 'available' ? { candidates: [], notice: statusText[data.status] } : ({
          notice: data.truncated ? '候选清单达到上限，请按 Unit 名称缩小搜索范围' : undefined,
          candidates: data.units.map((unit) => ({
            id: unit.name, name: unit.name, description: unit.active + ' / ' + unit.sub,
            disabled: monitors.data?.monitors.some((item) => item.unit === unit.name && item.enabled),
            add: () => request(monitorPath + '/' + encodeURIComponent(unit.name), { method: 'PUT' }),
          })),
        })} onClose={() => setAdding(false)} onExpired={onExpired}
        onAdded={(unit) => { setAdding(false); monitors.refresh(); setSelected(unit); }} />}
    </section>
  );
}

function UnitFacts({ unit }: { unit: Unit }) {
  return (
    <>
      <p className="observation">
        来源 · {unit.source} · 观测 <Timestamp value={unit.observed_at} /> · {unit.verified ? '已核实' : '未核实'}
      </p>
      <dl className="detail-facts asset-facts">
        <div><dt>Active / Sub</dt><dd>{unit.active} / {unit.sub}</dd></div>
        <div><dt>ExecStart</dt><dd className="technical">{unit.exec_start ?? '不可用'}</dd></div>
        <div><dt>WorkingDirectory</dt><dd className="technical">{unit.working_directory ?? '不可用'}</dd></div>
        <div><dt>FragmentPath</dt><dd className="technical">{unit.fragment_path ?? '不可用'}</dd></div>
      </dl>
    </>
  );
}
