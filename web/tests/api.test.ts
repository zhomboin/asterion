import { strict as assert } from 'node:assert';
import { test } from 'node:test';
import { ApiError, request } from '../src/api.ts';

test('accepts a bodyless 202 for operation confirmations', async (context) => {
  const fetch = context.mock.method(globalThis, 'fetch', async () => new Response(null, { status: 202 }));
  const result = await request('/operations/example/confirm', {
    method: 'POST',
    body: { challenge: 'fixture' },
    response: 'empty',
  });
  assert.equal(result, undefined);
  assert.equal(fetch.mock.calls.length, 1);
  const headers = fetch.mock.calls[0].arguments[1]?.headers as Headers;
  assert.equal(headers.get('X-Asterion-CSRF'), '1');
});

test('keeps JSON required for reads and reports errors for bodyless workflows', async (context) => {
  const fetch = context.mock.method(globalThis, 'fetch');
  fetch.mock.mockImplementationOnce(async () => new Response(null, { status: 200 }));
  await assert.rejects(request('/me'), SyntaxError);

  fetch.mock.mockImplementationOnce(async () => Response.json({ message: '确认已失效' }, { status: 409 }));
  await assert.rejects(
    request('/operations/example/confirm', { method: 'POST', response: 'empty' }),
    (error) => error instanceof ApiError && error.status === 409 && error.message === '确认已失效',
  );
});
