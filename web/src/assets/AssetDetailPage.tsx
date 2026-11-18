import { useState } from 'react';
import { errorMessage, isUnauthorized, request } from '../api';
import { Problem, Timestamp } from '../ui';
import { useResource } from '../useResource';
import { AssetForm } from './AssetForm';
import { AssetBindings } from './AssetBindings';
import { managementLabels, type AssetDetail } from './data';

export function AssetDetailPage({ id, onExpired }: { id: string; onExpired: () => void }) {
  const path = '/assets/' + encodeURIComponent(id);
  const asset = useResource<AssetDetail>(path, onExpired);
  const [editing, setEditing] = useState(false);
  const [removing, setRemoving] = useState(false);
  const [busy, setBusy] = useState(false);
  const [error, setError] = useState('');
  const data = asset.data;

  async function remove() {
    setBusy(true);
    setError('');
    try {
      await request<void>(path, { method: 'DELETE' });
      window.location.assign('/assets');
    } catch (failure) {
      if (isUnauthorized(failure)) onExpired();
      else setError(errorMessage(failure));
    } finally {
      setBusy(false);
    }
  }

  return (
    <>
      <a className="text-link back-link" href="/assets">服务资产</a>
      {asset.loading && <p role="status">正在读取资产…</p>}
      {asset.error && <Problem error={asset.error} retry={asset.refresh} />}
      {data && (
        <>
          <div className="page-heading">
            <h1>{data.name}</h1>
            {!editing && !removing && (
              <div className="actions">
                <button type="button" onClick={() => setEditing(true)}>编辑</button>
                <button type="button" className="danger quiet" onClick={() => setRemoving(true)}>删除资产</button>
              </div>
            )}
          </div>
          {editing ? (
            <AssetForm asset={data} onExpired={onExpired} onCancel={() => setEditing(false)} onSaved={() => {
              setEditing(false);
              asset.refresh();
            }} />
          ) : <AssetFacts asset={data} />}
          {removing && (
            <section className="detail-panel" aria-label="删除资产">
              <p>将删除资产 <strong>{data.name}</strong> 及其关联记录。</p>
              {error && <p className="error" role="alert">{error}</p>}
              <div className="dialog-actions">
                <button type="button" disabled={busy} onClick={() => { setError(''); setRemoving(false); }}>取消</button>
                <button type="button" disabled={busy} className="danger primary" onClick={() => void remove()}>确认删除</button>
              </div>
            </section>
          )}
          {!editing && !removing && <AssetBindings key={data.node_id} asset={data} onChanged={asset.refresh} onExpired={onExpired} />}
        </>
      )}
    </>
  );
}

function AssetFacts({ asset }: { asset: AssetDetail }) {
  return (
    <section className="monitor-view" aria-label="资产资料">
      {asset.description && <p className="prose-value">{asset.description}</p>}
      <p className="observation">
        人工登记 · {asset.verified_at == null ? '未核实' : <>核实 <Timestamp value={asset.verified_at} /></>}
      </p>
      {asset.tags.length > 0 && <ul className="tags" aria-label="标签">{asset.tags.map((tag, index) => <li key={index}>{tag}</li>)}</ul>}
      <dl className="detail-facts asset-facts">
        <div><dt>节点</dt><dd><a className="text-link" href={'/nodes/' + encodeURIComponent(asset.node_id)}>{asset.node_name}</a></dd></div>
        <div><dt>管理方式</dt><dd>{managementLabels[asset.management]}</dd></div>
        <div><dt>最后观测</dt><dd><Timestamp value={asset.last_seen_at} /></dd></div>
        <div><dt>编辑时间</dt><dd><Timestamp value={asset.updated_at} /></dd></div>
        <div><dt>部署路径</dt><dd className="technical">{asset.deploy_path || '未核实'}</dd></div>
        <div><dt>配置路径</dt><dd className="technical">{asset.config_path || '未核实'}</dd></div>
        <div><dt>仓库地址</dt><dd className="technical"><Repository value={asset.repo_url} /></dd></div>
        <div><dt>备注</dt><dd className="prose-value">{asset.notes || '—'}</dd></div>
      </dl>
    </section>
  );
}

function Repository({ value }: { value: string | null }) {
  if (!value) return <>未核实</>;
  return /^https?:\/\//i.test(value)
    ? <a className="text-link" href={value} target="_blank" rel="noreferrer">{value}</a>
    : <>{value}</>;
}
