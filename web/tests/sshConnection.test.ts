import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import { destination, parseDestination, readConnection } from '../src/sshConnection.ts';

test('保留 OpenSSH 别名的默认用户名和端口，并能编辑 IPv6 地址', () => {
  for (const profile of ['ssh://alias', 'ssh://user@host:22222', 'ssh://user@[::1]:22222']) {
    const { host, user, port } = parseDestination(profile);
    assert.equal(destination(host, user, port), profile);
  }
  assert.equal(destination('[::1]', '', ''), 'ssh://[::1]');
});

test('仅提交当前认证方式的字段，空密码表示保留原密码', () => {
  const form = new FormData();
  form.set('ssh_host', 'alias');
  form.set('ssh_method', 'password');
  assert.equal(readConnection(form).ssh_auth.password, undefined);
  form.set('ssh_password', ' fixture ');
  assert.equal(readConnection(form).ssh_auth.password, ' fixture ');
  form.set('ssh_method', 'key');
  form.set('identity_file', '~/.ssh/key');
  assert.deepEqual(readConnection(form).ssh_auth, { method: 'key', identity_file: '~/.ssh/key' });
});
