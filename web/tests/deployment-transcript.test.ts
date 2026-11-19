import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import { deploymentTranscript } from '../src/deploymentTranscript.ts';

test('prefixes each command once and preserves chronological output and line breaks', () => {
  const script = 'set -eu\nprintf "%s" "$value"\n$ continuation';
  const records = ['SSH 身份验证成功', '$ ' + script, '进度 100%\n', '$ uname -sm', 'Linux x86_64', '退出码 0'];
  assert.equal(deploymentTranscript(records, 'user@host'),
    ['SSH 身份验证成功', 'user@host:~# ' + script, '进度 100%\n', 'user@host:~# uname -sm', 'Linux x86_64', '退出码 0'].join('\n'));
});

test('uses a generic prompt without a saved target and preserves errors and empty logs', () => {
  assert.equal(deploymentTranscript(['$ uname']), 'ssh:~# uname');
  assert.equal(deploymentTranscript(['SSH 认证失败']), 'SSH 认证失败');
  assert.equal(deploymentTranscript([]), '');
});
