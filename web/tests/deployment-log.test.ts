import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import { groupDeploymentLog } from '../src/groupDeploymentLog.ts';

test('keeps multiline commands intact and groups their output in arrival order', () => {
  const command = 'set -eu\nprintf "%s" "$value"\n$ continuation';
  assert.deepEqual(groupDeploymentLog(['SSH 身份验证成功', '$ ' + command, '传输：100%', '退出码 0', '$ uname -sm'], false), [
    { kind: 'output', lines: ['SSH 身份验证成功'] },
    { kind: 'command', lines: [command] },
    { kind: 'output', lines: ['传输：100%', '退出码 0'] },
    { kind: 'command', lines: ['uname -sm'] },
  ]);
});

test('keeps the final failure distinct even if no command ran', () => {
  assert.deepEqual(groupDeploymentLog([], false), []);
  assert.deepEqual(groupDeploymentLog(['SSH 认证失败'], true), [{ kind: 'error', lines: ['SSH 认证失败'] }]);
  assert.deepEqual(groupDeploymentLog(['$ check', '退出码 1', '权限不足'], true).map(block => block.kind),
    ['command', 'output', 'error']);
});
