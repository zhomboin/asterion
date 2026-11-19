import { useState } from 'react';
import { Problem } from '../ui';
import { useResource } from '../useResource';
import { TerminalPanel } from '../terminal/TerminalPanel';
import { managedLabels, type ManagedChange, type NodeOperations } from './data';

type Group = 'packages' | 'configs' | 'containers';
type Profile = {
  image: string; restart: string; ports?: [number, number][];
  mounts?: { source?: string; volume?: string; destination: string }[];
  restart_choices?: string[]; port_choices?: [number, number][];
  mount_choices?: Record<string, { source?: string; volume?: string; destination: string }>;
};
type Entry = {
  key: string; revision: string; package: string; installed: boolean; command: string;
  path: string; unit: string; reload: string; content: string; name: string; engine: string;
  target: { id: string; profile: string | null; settings?: ManagedChange['settings'] } | null; running: boolean; terminal: boolean;
  profiles: Record<string, Profile>;
};
type Props = {
  base: string; nodeId: string; online: boolean; group: Group; controls?: NodeOperations; onExpired: () => void;
};

export function ManagedPanel({ base, nodeId, online, group, controls, onExpired }: Props) {
  const path = base + '/managed/' + group;
  const enabled = Boolean(online && controls?.catalog.managed?.includes(group));
  const list = useResource<{ keys: string[] }>(enabled ? path : null, onExpired);
  const [selected, setSelected] = useState('');
  const key = list.data?.keys.includes(selected) ? selected : '';
  const detail = useResource<Entry>(enabled && key ? path + '?' + new URLSearchParams({ key }) : null, onExpired);
  const terminal = useResource<{ enabled: boolean }>(
    enabled && group === 'containers' && detail.data?.terminal ? base + '/container-terminal/capability' : null, onExpired);
  const title = { packages: '软件安装', configs: '服务配置', containers: '容器服务' }[group];
  return <section className="monitor-section" aria-label={title}>
    <div className="section-bar"><h2>{title}</h2>
      {key && <button onClick={detail.refresh}>核对当前状态</button>}
    </div>
    {!enabled && <p className="empty">{online ? '节点未授权此操作' : '节点未连接'}</p>}
    {list.loading && <p role="status">正在读取授权清单…</p>}
    {list.error && <Problem error={list.error} retry={list.refresh} />}
    {enabled && list.data && (list.data.keys.length
      ? <label className="search-field">授权目标<select value={key} onChange={(event) => setSelected(event.target.value)}>
        <option value="">选择目标</option>{list.data.keys.map((item) => <option key={item}>{item}</option>)}
      </select></label> : <p className="empty">暂无授权目标</p>)}
    {detail.error && <Problem error={detail.error} retry={detail.refresh} />}
    {detail.loading && <p role="status">正在核对目标…</p>}
    {enabled && controls && key && detail.data?.key === key && !detail.error &&
      <EntryForm key={key + detail.data.revision} entry={detail.data} group={group} controls={controls}
        terminal={terminal.data?.enabled === true && !terminal.error} nodeId={nodeId} onExpired={onExpired} />}
  </section>;
}

function EntryForm({ entry, group, controls, terminal, nodeId, onExpired }: {
  entry: Entry; group: Group; controls: NodeOperations; terminal: boolean; nodeId: string; onExpired: () => void;
}) {
  const [text, setText] = useState(entry.content ?? '');
  const [profile, setProfile] = useState(entry.target?.profile ?? Object.keys(entry.profiles ?? {})[0] ?? '');
  const [consoleOpen, setConsoleOpen] = useState(false);
  const [settings, setSettings] = useState(entry.target?.settings);
  const [error, setError] = useState('');
  function propose(verb: ManagedChange['verb'], label: string, options: Partial<ManagedChange> = {}, content?: string) {
    controls.propose({ type: 'managed', change: { key: entry.key, revision: entry.revision, verb, ...options } }, label, content);
  }
  if (group === 'packages') return <div className="fields detail-panel">
    <h3>{entry.package} · {entry.installed ? '已安装' : '未安装'}</h3>
    <pre className="output">{entry.command}</pre>
    <button disabled={entry.installed} onClick={() => propose('install', entry.command)}>检查安装</button>
  </div>;
  if (group === 'configs') {
    const bytes = new TextEncoder().encode(text);
    async function prepare() {
      try {
        const hash = await crypto.subtle.digest('SHA-256', bytes);
        const digest = Array.from(new Uint8Array(hash), (byte) => byte.toString(16).padStart(2, '0')).join('');
        propose('save_reload', entry.path + ' → ' + entry.reload + ' ' + entry.unit, { digest }, text);
      } catch {
        setError('无法校验配置摘要，请使用受信任的 HTTPS 连接。');
      }
    }
    return <div className="fields detail-panel">
      <label>{entry.path}<textarea className="technical" rows={14} spellCheck={false} value={text}
        onChange={(event) => setText(event.target.value)} /></label>
      <p>{entry.reload === 'restart' ? '保存后重启' : '保存后重载'} · {entry.unit}</p>
      {bytes.length > 8192 && <p role="alert">配置不能超过 8192 字节</p>}
      {error && <p role="alert">{error}</p>}
      <button disabled={text === entry.content || bytes.length > 8192} onClick={() => void prepare()}>检查并保存</button>
    </div>;
  }
  const spec = entry.profiles[profile];
  const options = settings ?? { restart: spec?.restart ?? 'no', ports: [], mounts: [] };
  return <div className="fields detail-panel">
    <h3>{entry.name} · {entry.target ? entry.running ? '运行中' : '已停止' : '未创建'}</h3>
    {Object.keys(entry.profiles).length > 0 && <label>配置方案<select value={profile} onChange={(event) => {
      setProfile(event.target.value); setSettings(undefined);
    }}>
      {Object.keys(entry.profiles).map((option) => <option key={option}>{option}</option>)}</select></label>}
    {spec && <><dl className="detail-facts asset-facts">
      <div><dt>镜像</dt><dd>{spec.image}</dd></div><div><dt>重启策略</dt><dd>{spec.restart}</dd></div>
      <div><dt>本机端口 → 容器</dt><dd>{spec.ports?.map((port) => port.join(' → ')).join('、') || '无'}</dd></div>
      <div><dt>挂载</dt><dd>{spec.mounts?.map((mount) => (mount.volume ?? mount.source) + ' → ' + mount.destination
        + (mount.volume ? '（数据卷）' : '（只读）')).join('、') || '无'}</dd></div>
    </dl>
    {spec.restart_choices && <label>选择重启策略<select value={options.restart} onChange={(event) =>
      setSettings({ ...options, restart: event.target.value })}>
      {spec.restart_choices.map((rule) => <option key={rule}>{rule}</option>)}</select></label>}
    {spec.port_choices?.map((pair) => {
      const selected = options.ports.some((port) => port.join(':') === pair.join(':'));
      return <label key={pair.join(':')}><input type="checkbox" checked={selected} onChange={() =>
        setSettings({ ...options, ports: selected ? options.ports.filter((port) => port.join(':') !== pair.join(':')) : [...options.ports, pair] })} />
        端口 {pair.join(' → ')}</label>;
    })}
    {Object.entries(spec.mount_choices ?? {}).map(([id, mount]) =>
      <label key={id}><input type="checkbox" checked={options.mounts.includes(id)} onChange={() =>
        setSettings({ ...options, mounts: options.mounts.includes(id) ? options.mounts.filter((name) => name !== id) : [...options.mounts, id] })} />
        {mount.volume ?? mount.source} → {mount.destination}（{mount.volume ? '命名卷' : '只读'}）</label>)}
    {entry.target && <p>重建会替换容器并丢弃可写层；命名卷不删除，是否继续挂载取决于所选方案。</p>}
    <button onClick={() => propose(entry.target ? 'recreate' : 'create', entry.name + ' · ' + entry.engine, { profile, settings: options })}>
      {entry.target ? '检查重建' : '检查创建'}</button></>}
    {entry.target && <div className="row-actions">{(['start', 'stop', 'restart'] as const).map((verb) =>
      <button key={verb} disabled={verb === 'start' ? entry.running : !entry.running}
        onClick={() => propose(verb, entry.name + ' · ' + entry.target?.id)}>{managedLabels[verb]}</button>)}
      {terminal && entry.running && <button onClick={() => setConsoleOpen(!consoleOpen)}>{consoleOpen ? '收起终端' : '进入容器'}</button>}
    </div>}
    {consoleOpen && terminal && entry.running && entry.target && <TerminalPanel nodeId={nodeId}
      container={{ key: entry.key, id: entry.target.id, engine: entry.engine }} onExpired={onExpired} />}
  </div>;
}
