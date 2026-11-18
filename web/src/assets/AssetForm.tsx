import { useState, type FormEvent } from 'react';
import { errorMessage, isUnauthorized, request, type NodePage } from '../api';
import { Pagination, Problem } from '../ui';
import { useResource } from '../useResource';
import { managementOptions, type Asset, type AssetDetail, type AssetDraft } from './data';

export function AssetForm({ asset, onSaved, onCancel, onExpired }: {
  asset?: Asset;
  onSaved: (saved: AssetDetail) => void;
  onCancel: () => void;
  onExpired: () => void;
}) {
  const [error, setError] = useState('');
  const [busy, setBusy] = useState(false);
  const [nodeId, setNodeId] = useState(asset?.node_id ?? '');
  const [nodeQuery, setNodeQuery] = useState('');
  const [nodeOffset, setNodeOffset] = useState(0);
  const params = new URLSearchParams({ limit: '100', offset: String(nodeOffset) });
  if (nodeQuery) params.set('q', nodeQuery);
  const nodes = useResource<NodePage>('/nodes?' + params, onExpired);
  const selectedNodeMissing = Boolean(nodeId && !nodes.data?.nodes.some((node) => node.id === nodeId));

  async function submit(event: FormEvent<HTMLFormElement>) {
    event.preventDefault();
    const fields = new FormData(event.currentTarget);
    const value = (name: string) => String(fields.get(name) ?? '').trim();
    const body: AssetDraft = {
      node_id: nodeId,
      name: value('name'),
      management: value('management') as AssetDraft['management'],
      description: value('description'),
      repo_url: value('repo_url') || null,
      deploy_path: value('deploy_path') || null,
      config_path: value('config_path') || null,
      tags: [...new Set(value('tags').split(/\r?\n/u).map((tag) => tag.trim()).filter(Boolean))],
      notes: value('notes'),
      verified: fields.get('verified') === 'on',
    };
    setBusy(true);
    setError('');
    try {
      const path = asset ? '/assets/' + encodeURIComponent(asset.id) : '/assets';
      onSaved(await request<AssetDetail>(path, { method: asset ? 'PATCH' : 'POST', body }));
    } catch (failure) {
      if (isUnauthorized(failure)) onExpired();
      else setError(errorMessage(failure));
    } finally {
      setBusy(false);
    }
  }

  return (
    <form className="asset-form detail-panel fields" onSubmit={submit}>
      <h2>{asset ? '编辑资产' : '登记资产'}</h2>
      <div className="form-grid">
        <label>资产名称<input name="name" required maxLength={120} defaultValue={asset?.name} /></label>
        <label>管理方式<select name="management" defaultValue={asset?.management ?? 'other'}>
          {managementOptions.map(([value, label]) => <option key={value} value={value}>{label}</option>)}
        </select></label>
      </div>
      <div className="form-grid node-picker">
        <label>搜索节点<input type="search" value={nodeQuery} onChange={(event) => {
          setNodeQuery(event.target.value.trimStart());
          setNodeOffset(0);
        }} placeholder="名称或 ID" maxLength={100} /></label>
        <label>节点<select name="node_id" value={nodeId} onChange={(event) => setNodeId(event.target.value)} required>
          <option value="">选择节点</option>
          {selectedNodeMissing && <option value={nodeId}>{nodeId === asset?.node_id ? asset.node_name : nodeId}</option>}
          {nodes.data?.nodes.map((node) => <option key={node.id} value={node.id}>{node.name}</option>)}
        </select></label>
      </div>
      {nodes.loading && <p role="status">正在加载节点…</p>}
      {nodes.error && <Problem error={nodes.error} retry={nodes.refresh} />}
      <Pagination label="节点候选分页" offset={nodeOffset} pageSize={100} hasMore={nodes.data?.has_more} onChange={setNodeOffset} />
      <label>说明<textarea name="description" rows={2} maxLength={1500} defaultValue={asset?.description} /></label>
      <div className="form-grid">
        <label>部署路径<input name="deploy_path" maxLength={1024} defaultValue={asset?.deploy_path ?? ''} /></label>
        <label>配置路径<input name="config_path" maxLength={1024} defaultValue={asset?.config_path ?? ''} /></label>
      </div>
      <label>仓库地址<input name="repo_url" maxLength={1024} defaultValue={asset?.repo_url ?? ''} /></label>
      <div className="form-grid">
        <label>标签<textarea name="tags" rows={2} placeholder="每行一个标签" defaultValue={asset?.tags.join('\n')} /></label>
        <label>备注<textarea name="notes" rows={2} maxLength={1500} defaultValue={asset?.notes} /></label>
      </div>
      <label className="checkbox-field"><input type="checkbox" name="verified" defaultChecked={asset?.verified_at != null} /> 已人工核实</label>
      {error && <p role="alert" className="error">{error}</p>}
      <div className="dialog-actions">
        <button type="button" onClick={onCancel} disabled={busy}>取消</button>
        <button type="submit" className="primary" disabled={busy || !nodeId}>保存</button>
      </div>
    </form>
  );
}
