import type { RefreshWireOutcome } from '../auth/refreshOutcome';
export const ROTATION_CONFLICT_MESSAGE = 'Refresh token was concurrently rotated, please retry';
/** The deadline includes headers, body consumption and backoff. Every retry
 * presents the original credential to recover a lost rotation response. */
export async function requestRefreshWithRetry(url: string, refreshToken: string, options: {
  fetch?: typeof fetch; budgetMs?: number; attemptTimeoutMs?: number;
  backoffMs?: readonly number[]; now?: () => number;
} = {}): Promise<RefreshWireOutcome> {
  const send = options.fetch ?? fetch;
  const now = options.now ?? (() => performance.now());
  const deadline = now() + (options.budgetMs ?? 25000);
  const backoff = options.backoffMs ?? [250, 1000];
  let cause: unknown;
  for (let attempt = 0; attempt < 3 && now() < deadline; attempt++) {
    const controller = new AbortController();
    let timer: ReturnType<typeof setTimeout> | undefined;
    let retryAfter = 0;
    try {
      // Promise.race also bounds fetch implementations that fail to respect
      // abort during response body consumption.
      const outcome = await Promise.race([
        (async (): Promise<RefreshWireOutcome> => {
          const response = await send(url, {
            method: 'POST', headers: { Accept: 'application/json', 'Content-Type': 'application/json' },
            body: JSON.stringify({ refresh_token: refreshToken, client: 'mobile' }), signal: controller.signal,
          });
          const header = response.headers.get('Retry-After');
          if (header) {
            const seconds = Number(header);
            retryAfter = Math.max(0, Number.isFinite(seconds) ? seconds * 1000 : Date.parse(header) - Date.now());
          }
          let payload: unknown;
          try { payload = await response.json(); } catch (error) {
            // A terminal HTTP rejection remains definitive without a JSON body.
            if (response.status >= 400 && response.status < 500 && ![408, 429].includes(response.status))
              return { kind: 'rejected', status: response.status };
            return { kind: 'transient', cause: error };
          }
          const data = payload && typeof payload === 'object' ? payload as Record<string, unknown> : {};
          if (!response.ok) {
            const transient = response.status < 400 || response.status >= 500 || [408, 429].includes(response.status)
              || (response.status === 409 && typeof data.message === 'string'
                && data.message.replace(/^Conflict: /, '') === ROTATION_CONFLICT_MESSAGE);
            return transient ? { kind: 'transient', cause: response.status } : { kind: 'rejected', status: response.status };
          }
          if (typeof data.access_token !== 'string' || !data.access_token.trim()
            || typeof data.refresh_token !== 'string' || !data.refresh_token.trim()
            || (data.expires_in !== undefined && (typeof data.expires_in !== 'number' || !Number.isFinite(data.expires_in))))
            return { kind: 'transient', cause: 'incomplete_refresh_response' };
          return { kind: 'refreshed', credentials: { access_token: data.access_token,
            refresh_token: data.refresh_token, expires_in: data.expires_in as number | undefined } };
        })(),
        new Promise<never>((_, reject) => { timer = setTimeout(() => {
          controller.abort(); reject(new Error('refresh_timeout'));
        }, Math.min(options.attemptTimeoutMs ?? 7500, deadline - now())); }),
      ]);
      if (outcome.kind !== 'transient') return outcome;
      cause = outcome.cause;
    } catch (error) { cause = error; }
    finally { if (timer) clearTimeout(timer); controller.abort(); }
    if (attempt === 2) break;
    const delay = Math.max(backoff[attempt] ?? 0, retryAfter);
    if (delay >= deadline - now()) break;
    await new Promise((resolve) => setTimeout(resolve, delay));
  }
  return { kind: 'transient', cause };
}
