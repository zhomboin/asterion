import { useState, type FormEvent } from 'react';
import { errorMessage, isUnauthorized, request, type Node } from '../api';
import { alertMetric, type AlertMetric, type AlertRule, type RuleDraft } from './data';

export function AlertRuleForm({ nodes, rule, onExpired, onCancel, onSave }: {
  nodes: Node[];
  rule?: AlertRule;
  onExpired: () => void;
  onCancel: () => void;
  onSave: () => void;
}) {
  const [metric, setMetric] = useState<AlertMetric>(rule?.metric ?? 'offline');
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');

  async function save(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const values = new FormData(event.currentTarget);
    const data: RuleDraft = {
      node_id: String(values.get('node_id') ?? ''),
      metric,
      threshold: metric === 'offline' ? null : Number(values.get('threshold')),
      for_seconds: Number(values.get('for_seconds')),
      enabled: values.get('enabled') === 'on',
    };
    setBusy(true);
    setError('');
    try {
      const target = rule ? '/alert-rules/' + encodeURIComponent(rule.id) : '/alert-rules';
      await request<unknown>(target, { method: rule ? 'PATCH' : 'POST', body: data });
      onSave();
    } catch (failure) {
      if (isUnauthorized(failure)) onExpired();
      else setError(errorMessage(failure));
    } finally {
      setBusy(false);
    }
  }

  return (
    <form className="fields detail-panel asset-form" onSubmit={save}>
      <h2>{rule ? '编辑规则' : '添加规则'}</h2>
      <div className="form-grid">
        <label>节点<select name="node_id" defaultValue={rule?.node_id ?? ''} required>
          <option value="">选择节点</option>
          {rule && !nodes.some((node) => node.id === rule.node_id) && <option value={rule.node_id}>{rule.node_name}</option>}
          {nodes.map((node) => <option key={node.id} value={node.id}>{node.name}</option>)}
        </select></label>
        <label>指标<select value={metric} onChange={(event) => setMetric(event.target.value as AlertMetric)}>
          {Object.entries(alertMetric).map(([value, label]) => <option key={value} value={value}>{label}</option>)}
        </select></label>
      </div>
      <div className="form-grid">
        {metric !== 'offline' && <label>阈值 %<input type="number" name="threshold" min={0} max={100} step="any" defaultValue={rule?.threshold ?? 85} required /></label>}
        <label>持续秒数<input type="number" name="for_seconds" min={0} max={3600} step={1} defaultValue={rule?.for_seconds ?? 60} required /></label>
      </div>
      <label className="checkbox-field"><input type="checkbox" name="enabled" defaultChecked={rule?.enabled ?? true} /> 启用规则</label>
      {error && <p role="alert" className="error">{error}</p>}
      <div className="dialog-actions">
        <button type="button" disabled={busy} onClick={onCancel}>取消</button>
        <button type="submit" disabled={busy} className="primary">保存</button>
      </div>
    </form>
  );
}
