import { useState } from 'react';
import { formatTime, type Enrollment } from './api';

const agentPath = '/api/v1/agent/connect';

function endpoint(value: string) {
  try {
    const url = new URL(value.trim());
    if (!['https:', 'wss:'].includes(url.protocol) || url.username || url.password || url.search || url.hash
      || !['/', agentPath].includes(url.pathname)) return '';
    url.protocol = 'wss:';
    url.pathname = agentPath;
    return url.href;
  } catch {
    return '';
  }
}

const bootstrap = `[Service]
LoadCredential=enrollment:/etc/asterion-agent/enrollment
RuntimeDirectory=asterion-agent-bootstrap
RuntimeDirectoryMode=0700
ExecStart=
ExecStart=/bin/sh -ec '/usr/bin/install -m 0600 /run/credentials/%n/enrollment /run/asterion-agent-bootstrap/enrollment; exec /opt/asterion-agent/current/bin/asterion-agent'
Environment=ASTERION_AGENT_ENROLLMENT_FILE=/run/asterion-agent-bootstrap/enrollment`;

export function AgentEnrollment({ enrollment, reEnroll, onClose }: { enrollment: Enrollment; reEnroll: boolean; onClose: () => void }) {
  const [origin, setOrigin] = useState(window.location.origin);
  const [copied, setCopied] = useState<'config' | 'token'>();
  const [copyError, setCopyError] = useState(false);
  const url = endpoint(origin);
  const config = url ? 'ASTERION_SERVER_URL=' + url : '';

  async function copy(kind: 'config' | 'token', value: string) {
    try {
      await navigator.clipboard.writeText(value);
      setCopied(kind);
      setCopyError(false);
    } catch {
      setCopied(undefined);
      setCopyError(true);
    }
  }

  return <div className="fields">
    <p>凭证仅显示一次，{formatTime(enrollment.expires_at)} 到期。Agent 在线后，节点会自动更新状态。</p>
    <label>目标服务器可访问的中心地址
      <input type="url" value={origin} spellCheck={false} onChange={(event) => { setOrigin(event.target.value); setCopied(undefined); }} />
    </label>
    {!url && <p className="error">请输入 HTTPS 源地址或完整 WSS 接入地址。</p>}
    {url && <label>Agent 连接配置<input readOnly value={config} spellCheck={false} /></label>}
    <label>一次性注册凭证<textarea rows={2} readOnly spellCheck={false} value={enrollment.token} /></label>
    <details className="enrollment-guide">
      <summary>在目标服务器安装并启动 Agent</summary>
      <ol>
        <li>从与中心配套的发布包安装 <code>asterion-agent</code> 和 <code>asterion-agent.service</code>。完整安装步骤见部署文档的「安装和注册 Agent」。</li>
        <li>将连接配置写入 <code>/etc/asterion-agent/agent.env</code>。启用了私有 CA 时，在目标服务器配置对应 CA 证书。</li>
        <li>将一次性凭证写入权限为 <code>0600</code> 的 <code>/etc/asterion-agent/enrollment</code>，并在 <code>/etc/systemd/system/asterion-agent.service.d/10-enrollment.conf</code> 填写以下内容。</li>
      </ol>
      <pre>{bootstrap}</pre>
      {reEnroll && <>
        <p>在已有 Agent 的服务器上，先停止服务并备份旧身份文件。首次接入可跳过此步。</p>
        <pre>{'sudo systemctl stop asterion-agent.service\nsudo install -d -m 0700 /var/backups/asterion-agent\nsudo mv /var/lib/asterion-agent/identity.json "/var/backups/asterion-agent/identity-$(date -u +%Y%m%dT%H%M%SZ).json"'}</pre>
      </>}
      <p>加载配置并启动 Agent。</p>
      <pre>{'sudo systemctl daemon-reload\nsudo systemctl enable asterion-agent.service\nsudo systemctl restart asterion-agent.service\nsudo systemctl status --no-pager asterion-agent.service'}</pre>
    </details>
    {copyError && <p className="error" role="alert">无法访问剪贴板，请手动复制。</p>}
    <div className="dialog-actions">
      <button type="button" disabled={!url} onClick={() => void copy('config', config)}>{copied === 'config' ? '已复制配置' : '复制配置'}</button>
      <button type="button" onClick={() => void copy('token', enrollment.token)}>{copied === 'token' ? '已复制凭证' : '复制凭证'}</button>
      <button className="primary" type="button" onClick={onClose}>完成</button>
    </div>
  </div>;
}
