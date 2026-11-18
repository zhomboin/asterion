import { useState } from 'react';
import { errorMessage, isUnauthorized, request } from '../api';
import { DataTable, PageHeader, Problem, Timestamp } from '../ui';
import { useResource } from '../useResource';
import { operationName, operationStates, operationTarget, type OperationDetail } from './data';

const auditLabels: Record<string, string> = {
  prepared: '创建确认', confirmed: '确认执行', queued: '加入队列', dispatched: '已下发', started: '开始执行', succeeded: '执行成功',
  failed: '执行失败', timed_out: '超时', cancelled: '取消', disconnected: '连接中断', expired: '确认过期',
  interrupted: '执行中断', cancel_requested: '请求取消',
};

const accountErrors: Record<string, string> = {
  agent_account_conflict: '账号、家目录或公钥已存在，或授权文件已变化。请刷新后核对。',
  agent_account_not_found: '节点上找不到所需的账号、目录或公钥。',
};

export function OperationDetailPage({ id, onExpired }: { id: string; onExpired: () => void }) {
  const path = '/operations/' + encodeURIComponent(id);
  const resource = useResource<OperationDetail>(path, onExpired, 2_000);
  const [confirming, setConfirming] = useState(false);
  const [cancelling, setCancelling] = useState(false);
  const [error, setError] = useState('');
  const job = resource.data;
  const cancellable = job?.can_cancel === true && !resource.error;
  const accountError = job?.error_code ? accountErrors[job.error_code] : undefined;

  async function cancel() {
    setError('');
    setCancelling(true);
    try {
      await request(path + '/cancel', { method: 'POST', response: 'empty' });
      setConfirming(false);
      resource.refresh();
    } catch (failure) {
      if (isUnauthorized(failure)) onExpired();
      else setError(errorMessage(failure));
    } finally {
      setCancelling(false);
    }
  }

  return (
    <>
      {!job && <PageHeader back={{ href: '/operations', label: '运维任务' }} title="任务详情" />}
      {resource.error && <Problem error={resource.error} retry={resource.refresh} />}
      {resource.loading && <p role="status">正在读取任务…</p>}
      {job && (
        <>
          <PageHeader
            back={{ href: '/operations', label: '运维任务' }}
            title={operationName(job.action)}
            actions={<span className="operation-state" data-state={job.state}>{operationStates[job.state] ?? job.state}</span>}
          />
          <dl className="detail-facts asset-facts">
            <div><dt>节点</dt><dd><a href={'/nodes/' + encodeURIComponent(job.node_id)} className="text-link">{job.node_name}</a></dd></div>
            <div><dt>目标</dt><dd className="technical">{operationTarget(job.action)}</dd></div>
            <div><dt>任务 ID</dt><dd className="technical">{job.id}</dd></div>
            <div><dt>执行人</dt><dd>{job.actor}</dd></div>
            <div><dt>创建时间</dt><dd><Timestamp value={job.created_at} /></dd></div>
            <div><dt>开始时间</dt><dd><Timestamp value={job.started_at} /></dd></div>
            <div><dt>结束时间</dt><dd><Timestamp value={job.finished_at} /></dd></div>
            <div><dt>耗时</dt><dd>{job.duration_ms == null ? '—' : job.duration_ms + ' ms'}</dd></div>
            {job.cancel_requested_at != null && <div><dt>请求取消时间</dt><dd><Timestamp value={job.cancel_requested_at} /></dd></div>}
            <div><dt>退出码</dt><dd>{job.exit_code ?? '—'}</dd></div>
            <div><dt>错误码</dt><dd className="technical">{job.error_code || '—'}</dd></div>
            {accountError && <div><dt>失败原因</dt><dd>{accountError}</dd></div>}
          </dl>
          {cancellable && (
            <div className="actions">
              {!confirming && <button type="button" onClick={() => setConfirming(true)}>取消任务</button>}
              {confirming && (
                <>
                  <button type="button" onClick={() => { setConfirming(false); setError(''); }} disabled={cancelling}>返回</button>
                  <button type="button" className="danger primary" onClick={() => void cancel()} disabled={cancelling}>确认取消</button>
                </>
              )}
            </div>
          )}
          {error && <p role="alert" className="error">{error}</p>}
          <section className="monitor-section" aria-label="任务输出">
            <h2>任务输出</h2>
            {job.truncated && <p className="observation">输出已截断</p>}
            {job.output
              ? <pre className="output" tabIndex={0}>{job.output}</pre>
              : <p className="empty">暂无输出</p>}
          </section>
          <section className="monitor-section" aria-label="任务审计">
            <h2>任务审计</h2>
            {job.audit.length ? (
              <DataTable label="任务审计" columns={['事件', '执行人', '时间']}>
                {job.audit.map((entry, index) => (
                  <tr key={index}>
                    <th scope="row">{auditLabels[entry.action] || entry.action}</th>
                    <td>{entry.actor ?? '系统'}</td><td><Timestamp value={entry.observed_at} /></td>
                  </tr>
                ))}
              </DataTable>
            ) : <p className="empty">暂无审计事件</p>}
          </section>
        </>
      )}
    </>
  );
}
