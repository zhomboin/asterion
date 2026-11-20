import { useEffect, useState, type FormEvent } from 'react';
import type { Node } from '../api';
import { Problem } from '../ui';
import { useResource } from '../useResource';
import { ProposalDialog } from './ProposalDialog';
import { fileErrors, operationStates, type ManagedChange, type OperationCatalog, type OperationDetail } from './data';

type Selection = { group: 'files' | 'nginx'; path: string };
type FileData = { path: string; resolved: string; content: string; revision: string };
type Catalog = { data?: OperationCatalog; error?: string; loading: boolean; refresh: () => void };

export function FilePanel({ base, node, online, catalog, onExpired, dirty, setDirty }: {
  base: string; node: Node; online: boolean; catalog: Catalog; onExpired: () => void;
  dirty: boolean; setDirty: (value: boolean) => void;
}) {
  const [path, setPath] = useState('');
  const [selected, setSelected] = useState<Selection>();
  const [job, setJob] = useState('');
  const [outcome, setOutcome] = useState<OperationDetail>();
  const [submittedRevision, setSubmittedRevision] = useState('');
  const [openCount, setOpenCount] = useState(0);
  const enabled = online && catalog.data?.file_editor && !catalog.error;
  const configs = useResource<{ paths: string[] }>(enabled ? base + '/managed/nginx' : null, onExpired);
  const detail = useResource<FileData>(selected
    ? base + '/managed/' + selected.group + '?' + new URLSearchParams({ key: selected.path }) : null, onExpired);
  const task = useResource<OperationDetail>(job ? '/operations/' + encodeURIComponent(job) : null, onExpired, 800);
  const finished = task.data;
  useEffect(() => {
    if (!dirty) return;
    const beforeUnload = (event: BeforeUnloadEvent) => event.preventDefault();
    window.addEventListener('beforeunload', beforeUnload);
    return () => window.removeEventListener('beforeunload', beforeUnload);
  }, [dirty]);
  useEffect(() => {
    if (!finished?.finished_at) return;
    setOutcome(finished);
    setJob('');
    if (finished.state === 'succeeded') {
      setDirty(false);
      detail.refresh();
      if (selected?.group === 'nginx') configs.refresh();
    }
  }, [finished]); // 只在本次任务结束时重新读取。

  function open(next: Selection) {
    if (!enabled || job || (dirty && !window.confirm('放弃当前未保存的修改？'))) return;
    setDirty(false);
    setOutcome(undefined);
    setOpenCount((value) => value + 1);
    setSelected(next);
    if (next.path === selected?.path && next.group === selected.group) detail.refresh();
  }
  function submit(event: FormEvent) {
    event.preventDefault();
    open({ group: 'files', path });
  }
  return <section className="monitor-section" aria-label="文件编辑">
    {!enabled && (catalog.error ? <Problem error={catalog.error} retry={catalog.refresh} />
      : <p className="empty">{!online ? '节点未连接' : catalog.loading ? '正在读取节点能力…' : '此节点未启用文件编辑'}</p>)}
    {enabled && <><div className="section-bar"><h2>Nginx 配置文件</h2><button onClick={configs.refresh}>刷新清单</button></div>
    {configs.loading && <p role="status">正在读取 Nginx 配置…</p>}
    {configs.error && <Problem error={configs.error} retry={configs.refresh} />}
    {configs.data && !configs.error && <label className="search-field">当前配置引用的文件
      <select disabled={Boolean(job)} value={selected?.group === 'nginx' ? selected.path : ''}
        onChange={(event) => open({ group: 'nginx', path: event.target.value })}>
        <option value="" disabled>选择文件（{configs.data.paths.length}）</option>
        {configs.data.paths.map((name) => <option key={name}>{name}</option>)}
      </select></label>}
    <form className="file-path-form" onSubmit={submit}>
      <label>手动打开文件<input required pattern="/.*" value={path} placeholder="远程文件的绝对路径"
        spellCheck={false} autoComplete="off" onChange={(event) => setPath(event.target.value)} /></label>
      <button disabled={Boolean(job) || !path.startsWith('/')}>打开</button>
    </form></>}
    {detail.error && <Problem error={detail.error} retry={detail.refresh} />}
    {detail.loading && <p role="status">正在读取文件…</p>}
    {selected && detail.data && <Editor key={openCount + selected.group + detail.data.revision}
      file={detail.data} nginx={selected.group === 'nginx'} node={node} setDirty={setDirty}
      busy={Boolean(!enabled || job || detail.error || (outcome?.state === 'succeeded' && submittedRevision === detail.data.revision))}
      reload={() => open(selected)} onConfirmed={(id) => {
        setSubmittedRevision(detail.data!.revision); setOutcome(undefined); setJob(id);
      }} onExpired={onExpired} />}
    {task.error && <Problem error={task.error} retry={task.refresh} />}
    {(job || outcome) && <div role="status">
      <a className="text-link" href={'/operations/' + (job || outcome?.id)}>保存任务</a>
      {' · '}{outcome ? operationStates[outcome.state] : '处理中'}
      {outcome && <p>{outcome.output || fileErrors[outcome.error_code ?? ''] || outcome.error_code}</p>}
    </div>}
  </section>;
}

function Editor({ file, nginx, node, busy, setDirty, reload, onConfirmed, onExpired }: {
  file: FileData; nginx: boolean; node: Node; busy: boolean; setDirty: (value: boolean) => void;
  reload: () => void; onConfirmed: (id: string) => void; onExpired: () => void;
}) {
  const initial = file.content.replace(/\r\n?/gu, '\n');
  const eol = file.content.match(/\r\n|\r|\n/u)?.[0] ?? '\n';
  const mixed = file.content.split(eol).some((part) => /[\r\n]/u.test(part));
  const [text, setText] = useState(initial);
  const [change, setChange] = useState<ManagedChange>();
  const [error, setError] = useState('');
  const content = text.replaceAll('\n', eol);
  const bytes = new TextEncoder().encode(content);
  const label = nginx ? '保存并重载 Nginx' : '保存文件';
  async function prepare() {
    try {
      const hash = await crypto.subtle.digest('SHA-256', bytes);
      const digest = Array.from(new Uint8Array(hash), (v) => v.toString(16).padStart(2, '0')).join('');
      setChange({ key: file.path, verb: nginx ? 'save_nginx' : 'save_text', revision: file.revision, digest });
    } catch {
      setError('无法计算文件摘要，请使用本机面板地址。');
    }
  }
  return <div className="fields detail-panel">
    <label>{file.path}<textarea className="technical" rows={16} spellCheck={false} value={text} readOnly={busy}
      onChange={(event) => { setText(event.target.value); setDirty(event.target.value !== initial); }} /></label>
    {file.resolved !== file.path && <p className="technical">链接目标 · {file.resolved}</p>}
    {mixed && <p>文件使用多种换行符，保存时将统一为首行的格式。</p>}
    {bytes.length > 65536 && <p role="alert">文件不能超过 64 KiB</p>}
    {error && <p role="alert">{error}</p>}
    <div className="row-actions"><button className="primary" disabled={busy || text === initial || bytes.length > 65536}
      onClick={() => void prepare()}>{label}</button><button onClick={reload}>重新读取</button></div>
    {change && <ProposalDialog nodeId={node.id} nodeName={node.name} action={{ type: 'managed', change }} content={content}
      onClose={() => setChange(undefined)} onConfirmed={(id) => { setChange(undefined); onConfirmed(id); }} onExpired={onExpired} />}
  </div>;
}
