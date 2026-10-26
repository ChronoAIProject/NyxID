import assert from 'node:assert/strict';
import { test } from 'node:test';
import { requestRefreshWithRetry, ROTATION_CONFLICT_MESSAGE } from './refreshRequest';
const valid = { access_token: 'successor-access', refresh_token: 'successor-refresh', expires_in: 900 };
const response = (status: number, body: unknown) => new Response(JSON.stringify(body), { status });
for (const first of ['conflict', '503', 'network', 'malformed', 'missing_refresh', 'bad_expiry']) {
  test(`refresh retries ${first} with the original token`, async () => {
    const sent: string[] = [];
    const fetcher: typeof fetch = async (_url, init) => {
      sent.push(JSON.parse(String(init?.body)).refresh_token);
      if (sent.length === 1) {
        if (first === 'network') throw new TypeError('connection lost');
        if (first === 'conflict') return response(409, { message: `Conflict: ${ROTATION_CONFLICT_MESSAGE}` });
        if (first === '503') return response(503, {});
        if (first === 'malformed') return new Response('{', { status: 200 });
        if (first === 'missing_refresh') return response(200, { access_token: 'partial' });
        return response(200, { ...valid, expires_in: 'bad' });
      }
      return response(200, valid);
    };
    const result = await requestRefreshWithRetry('http://example.invalid', 'original', { fetch: fetcher, backoffMs: [0, 0] });
    assert.equal(result.kind, 'refreshed'); assert.deepEqual(sent, ['original', 'original']);
  });
}
for (const status of [400, 401, 403, 409]) {
  test(`terminal ${status} never retries`, async () => {
    let count = 0;
    const result = await requestRefreshWithRetry('http://example.invalid', 'original', {
      fetch: async () => { count++; return response(status, { message: 'consent changed or expired' }); }, backoffMs: [0, 0],
    });
    assert.equal(result.kind, 'rejected'); assert.equal(count, 1);
  });
}
for (const boundary of ['headers', 'body', 'malformed', '503', 'unexpected_status']) {
  test(`persistent ${boundary} is transient and bounded`, { timeout: 2000 }, async () => {
    let count = 0;
    const fetcher: typeof fetch = async () => {
      count++;
      if (boundary === 'headers') return new Promise(() => {});
      if (boundary === 'body') return new Response(new ReadableStream({ start() {} }));
      if (boundary === 'unexpected_status') return response(302, {});
      return boundary === '503' ? response(503, {}) : new Response('{');
    };
    const start = performance.now();
    const result = await requestRefreshWithRetry('http://example.invalid', 'original', {
      fetch: fetcher, budgetMs: 80, attemptTimeoutMs: 20, backoffMs: [0, 0],
    });
    assert.equal(result.kind, 'transient'); assert.equal(count, 3);
    assert.ok(performance.now() - start < 500);
  });
}
test('Retry-After beyond budget returns transient without waiting', async () => {
  let count = 0;
  const result = await requestRefreshWithRetry('http://example.invalid', 'original', {
    fetch: async () => { count++; return new Response('{}', { status: 429, headers: { 'Retry-After': '120' } }); },
    budgetMs: 100, backoffMs: [0, 0],
  });
  assert.equal(result.kind, 'transient'); assert.equal(count, 1);
});
for (const status of [401, 403, 409]) {
  test(`malformed terminal ${status} is rejected without retries`, async () => {
    let count = 0;
    const result = await requestRefreshWithRetry('http://example.invalid', 'original', {
      fetch: async () => { count++; return new Response('{', { status }); }, backoffMs: [0, 0],
    });
    assert.equal(result.kind, 'rejected'); assert.equal(count, 1);
  });
}
