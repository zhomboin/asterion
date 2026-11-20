import { useState } from 'react';
import type { Node } from './api';
import { destination, parseDestination } from './sshConnection';

export function SshConnectionFields({ node }: { node?: Node | null }) {
  const saved = parseDestination(node?.ssh_profile);
  const [host, setHost] = useState(saved.host);
  const [user, setUser] = useState(saved.user);
  const [port, setPort] = useState(saved.port);
  const [method, setMethod] = useState<string>(node?.ssh_auth?.method ?? 'default');
  const keepPassword = node?.ssh_auth?.method === 'password' && destination(host, user, port) === node.ssh_profile;

  return <>
    <label>主机地址<input name="ssh_host" required maxLength={253} value={host}
      onChange={event => setHost(event.target.value)} placeholder="IP、域名或 OpenSSH 别名" autoCapitalize="none" spellCheck={false} /></label>
    <label>SSH 端口<input name="ssh_port" type="number" min={1} max={65535} value={port}
      onChange={event => setPort(event.target.value)} placeholder="使用 SSH 配置，默认 22" /></label>
    <label>SSH 用户名<input name="ssh_user" value={user} maxLength={64}
      onChange={event => setUser(event.target.value)} placeholder="留空使用本机 SSH 配置" autoCapitalize="none" spellCheck={false} /></label>
    <label>认证方式<select name="ssh_method" value={method} onChange={event => setMethod(event.target.value)}>
      <option value="default">使用本机 SSH 配置</option>
      <option value="password">服务器密码</option>
      <option value="key">指定 SSH 私钥</option>
    </select></label>
    {method === 'password' && <>
      <label>服务器密码<input type="password" name="ssh_password" required={!keepPassword}
        maxLength={4096} autoComplete="new-password" aria-describedby="ssh-password-storage"
        placeholder={keepPassword ? '留空保留已保存的密码' : undefined} /></label>
      <small className="muted" id="ssh-password-storage">密码以明文保存在本机私有数据库中。</small>
    </>}
    {method === 'key' && <label>本机私钥路径<input name="identity_file" required
      defaultValue={node?.ssh_auth?.identity_file} placeholder="~/.ssh/id_ed25519" maxLength={4096} autoCapitalize="none" spellCheck={false} /></label>}
    {!node?.ssh_profile && <p className="muted">首次连接请先在同一 Linux 或 WSL 用户的终端用 SSH 登录一次，核对服务器主机指纹。</p>}
  </>;
}
