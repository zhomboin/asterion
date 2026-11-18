import { useState } from 'react';
import { errorMessage, isUnauthorized, request, type Node } from '../api';
import { DataTable, fraction, Pagination, Problem } from '../ui';
import { useResource } from '../useResource';
import { AlertRuleForm } from './AlertRuleForm';
import { alertMetric, type AlertRule, type Rules } from './data';

export function AlertRulesPanel({ nodes, onExpired }: { nodes: Node[]; onExpired: () => void }) {
  const [offset, setOffset] = useState(0);
  const pageSize = 12;
  const rules = useResource<Rules>('/alert-rules?limit=' + pageSize + '&offset=' + offset, onExpired);
  const [edit, setEdit] = useState<{ data?: AlertRule }>();
  const [removing, setRemoving] = useState<AlertRule>();
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');

  async function remove(id: string) {
    setBusy(true);
    setError('');
    try {
      await request<void>('/alert-rules/' + encodeURIComponent(id), { method: 'DELETE' });
      setRemoving(undefined);
      rules.refresh();
    } catch (failure) {
      if (isUnauthorized(failure)) onExpired();
      else setError(errorMessage(failure));
    } finally {
      setBusy(false);
    }
  }

  return (
    <section aria-label="告警规则">
      <div className="section-bar">
        <h2>告警规则</h2>
        <button type="button" className="primary" onClick={() => setEdit({})} disabled={Boolean(edit || removing)}>添加规则</button>
      </div>
      {edit && (
        <AlertRuleForm
          key={edit.data?.id ?? 'new'}
          nodes={nodes}
          rule={edit.data}
          onExpired={onExpired}
          onCancel={() => setEdit(undefined)}
          onSave={() => { setEdit(undefined); rules.refresh(); }}
        />
      )}
      {rules.loading && <p role="status">正在读取规则…</p>}
      {rules.error && <Problem error={rules.error} retry={rules.refresh} />}
      {rules.data && !rules.data.rules.length && <p className="empty">暂无告警规则</p>}
      {Boolean(rules.data?.rules.length) && (
        <DataTable label="告警规则" columns={['指标', '节点', '阈值', '持续', '状态', '操作']}>
          {rules.data?.rules.map((rule) => (
            <tr key={rule.id}>
              <th scope="row">{alertMetric[rule.metric]}</th>
              <td><a className="text-link" href={'/nodes/' + encodeURIComponent(rule.node_id)}>{rule.node_name}</a></td>
              <td>{fraction(rule.threshold)}</td><td>{rule.for_seconds} 秒</td><td>{rule.enabled ? '启用' : '停用'}</td>
              <td><div className="actions">
                <button type="button" className="quiet" disabled={Boolean(edit || removing)} onClick={() => setEdit({ data: rule })}>编辑</button>
                <button type="button" className="danger quiet" disabled={Boolean(edit || removing)} onClick={() => setRemoving(rule)}>删除</button>
              </div></td>
            </tr>
          ))}
        </DataTable>
      )}
      <Pagination label="告警规则分页" offset={offset} pageSize={pageSize} hasMore={rules.data?.has_more} onChange={setOffset} />
      {removing && (
        <div className="monitor-section detail-panel">
          <p>将删除 {removing.node_name} 的{alertMetric[removing.metric]}规则。</p>
          {error && <p className="error" role="alert">{error}</p>}
          <div className="dialog-actions">
            <button type="button" onClick={() => { setRemoving(undefined); setError(''); }} disabled={busy}>取消</button>
            <button type="button" className="danger primary" onClick={() => void remove(removing.id)} disabled={busy}>确认删除</button>
          </div>
        </div>
      )}
    </section>
  );
}
