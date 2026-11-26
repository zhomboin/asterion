import { useEffect, useState, type FormEvent } from 'react';
import type { Node } from '../api';
import { Problem } from '../ui';
import { useResource } from '../useResource';
import { operationStates, type CollectorSettings, type OperationAction, type OperationDetail } from '../operations/data';
import { ProposalDialog } from '../operations/ProposalDialog';

type Deployment = {
  arch: string;
  state: 'active' | 'inactive' | 'missing' | 'unsupported';
  revision: string;
  root: boolean;
  enabled: boolean;
  settings: CollectorSettings | null;
  artifact_available: boolean;
};
const states = { active: '运行中', inactive: '已安装，未运行', missing: '未安装', unsupported: '缺少 systemd 部署环境' };

export function CollectorDeployment({ node, onExpired }: { node: Node; onExpired: () => void }) {
  const base = '/nodes/' + encodeURIComponent(node.id);
  const status = useResource<Deployment>(base + '/collector-deployment', onExpired);
  const [custom, setCustom] = useState(false);
  const [proposal, setProposal] = useState<OperationAction>();
  const [jobId, setJobId] = useState('');
  const [watching, setWatching] = useState(false);
  const result = useResource<OperationDetail>(jobId ? '/operations/' + jobId : null, onExpired, watching ? 1500 : 0);
  const job = result.data;
  useEffect(() => {
    if (watching && job?.finished_at) {
      setWatching(false);
      status.refresh();
    }
  }, [watching, job?.finished_at]); // 完成一次任务后重新核对节点状态。

  const info = status.data;
  const canDeploy = info && !status.error && info.state !== 'unsupported' && info.artifact_available;
  function prepare(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    if (!canDeploy || watching) return;
    const fields = new FormData(event.currentTarget);
    const settings = custom ? {
      interval_secs: Number(fields.get('interval')), retention_days: Number(fields.get('days')), max_mib: Number(fields.get('mib')),
    } : undefined;
    setProposal({ type: 'collector_deploy', revision: info.revision, settings });
  }

  return <section className="monitor-section" aria-label="服务器端采集器">
    <div className="section-bar">
      <h2>服务器端采集器</h2>
      <button type="button" onClick={status.refresh} disabled={watching || status.loading}>检查状态</button>
    </div>
    {status.loading && <p role="status">正在通过 SSH 检查节点…</p>}
    {status.error && <Problem error={status.error} retry={status.refresh} />}
    {info && <>
      <dl className="detail-facts">
        <div><dt>服务状态</dt><dd>{states[info.state]}</dd></div>
        {(info.state === 'active' || info.state === 'inactive') &&
          <div><dt>开机启动</dt><dd>{info.enabled ? '已启用' : '未启用'}</dd></div>}
        <div><dt>节点架构</dt><dd>{info.arch}</dd></div>
      </dl>
      {info.state !== 'unsupported' && !info.artifact_available && <p role="alert">面板运行包缺少适用于 {info.arch} 的采集器。</p>}
      {canDeploy && <form className="fields" onSubmit={prepare}>
        <fieldset className="node-fields" disabled={watching}>
          <label className="checkbox-field"><input type="checkbox" checked={custom} onChange={e => setCustom(e.target.checked)} /> 自定义采集参数</label>
          {custom && <>
            <label>采样间隔（秒）<input type="number" name="interval" min={5} max={3600} defaultValue={info.settings?.interval_secs} required /></label>
            <label>保留时间（天）<input type="number" name="days" min={1} max={365} defaultValue={info.settings?.retention_days} required /></label>
            <label>缓存上限（MiB）<input type="number" name="mib" min={1} max={16384} defaultValue={info.settings?.max_mib} required /></label>
          </>}
          <div className="actions"><button type="submit" className="primary">{info.state === 'missing' ? '部署采集器' : '重新部署采集器'}</button></div>
        </fieldset>
      </form>}
    </>}
    {result.error && <Problem error={result.error} retry={result.refresh} />}
    {jobId && <div role="status">
      <a className="text-link" href={'/operations/' + jobId}>部署任务</a> · {job ? operationStates[job.state] : '读取中'}
      {job?.output && <p>{job.output}</p>}
      {job?.error_code && !job.output && <p>请打开部署任务查看失败原因。</p>}
    </div>}
    {proposal && <ProposalDialog action={proposal} nodeId={node.id} nodeName={node.name} onExpired={onExpired}
      sudoForCollector={!info?.root} onClose={() => setProposal(undefined)}
      onConfirmed={id => { setJobId(id); setWatching(true); setProposal(undefined); }} />}
  </section>;
}
