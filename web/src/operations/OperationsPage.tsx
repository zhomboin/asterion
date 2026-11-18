import { useState, type FormEvent } from 'react';
import type { NodePage } from '../api';
import { DataTable, PageHeader, Pagination, Problem, Timestamp } from '../ui';
import { useResource } from '../useResource';
import { operationName, operationStates, operationTarget, type OperationList } from './data';

const pageSize = 12;

export function OperationsPage({ onExpired }: { onExpired: () => void }) {
  const initialNode = new URLSearchParams(window.location.search).get('node_id') || '';
  const [nodeId, setNodeId] = useState(initialNode);
  const [offset, setOffset] = useState(0);
  const params = new URLSearchParams({ limit: String(pageSize), offset: String(offset) });
  if (nodeId) params.set('node_id', nodeId);
  const jobs = useResource<OperationList>('/operations?' + params, onExpired, 4_000);
  const nodes = useResource<NodePage>('/nodes?limit=100', onExpired);

  function search(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    setNodeId(String(new FormData(event.currentTarget).get('node_id') ?? ''));
    setOffset(0);
  }

  return (
    <>
      <PageHeader title="运维任务" scope="受控操作的确认、执行与审计记录" />
      <form className="filter-bar operation-filters" onSubmit={search}>
        <label>节点<select name="node_id" defaultValue={initialNode}>
          <option value="">全部节点</option>
          {initialNode && !nodes.data?.nodes.some((node) => node.id === initialNode) && <option value={initialNode}>{initialNode}</option>}
          {nodes.data?.nodes.map((node) => <option key={node.id} value={node.id}>{node.name}</option>)}
        </select></label>
        <button type="submit">筛选</button>
      </form>
      {nodes.error && <Problem error={nodes.error} retry={nodes.refresh} />}
      {jobs.error && <Problem error={jobs.error} retry={jobs.refresh} />}
      {jobs.loading && <p role="status">正在读取运维任务…</p>}
      {jobs.data && jobs.data.operations.length === 0 && <p className="empty">暂无运维任务</p>}
      {Boolean(jobs.data?.operations.length) && (
        <DataTable label="运维任务" columns={['任务', '节点', '状态', '执行人', '创建时间']}>
          {jobs.data?.operations.map((job) => (
            <tr key={job.id}>
              <th scope="row">
                <a className="text-link" href={'/operations/' + encodeURIComponent(job.id)}>{operationName(job.action)}</a>
                <div className="table-secondary technical">{operationTarget(job.action)}</div>
              </th>
              <td><a className="text-link" href={'/nodes/' + encodeURIComponent(job.node_id)}>{job.node_name}</a></td>
              <td><span className="operation-state" data-state={job.state}>{operationStates[job.state] ?? job.state}</span></td>
              <td>{job.actor}</td><td><Timestamp value={job.created_at} /></td>
            </tr>
          ))}
        </DataTable>
      )}
      <Pagination label="运维任务分页" offset={offset} pageSize={pageSize} hasMore={jobs.data?.has_more} onChange={setOffset} />
    </>
  );
}
