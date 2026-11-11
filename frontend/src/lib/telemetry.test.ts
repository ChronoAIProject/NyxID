import { describe, it, expect, vi, beforeEach } from 'vitest';

const state = vi.hoisted(() => ({
  actor: 'account-a' as string | null,
  device: { accountId: 'account-a', enabled: true, revision: 0 },
  api: vi.fn(),
}));
vi.mock('./api-client', () => ({ apiClient: state.api }));
vi.mock('@/stores/auth-store', () => ({ useAuthStore: { getState: () => ({ user: state.actor ? { id: state.actor } : null }) } }));
vi.mock('@/stores/consent-store', () => ({ useConsentStore: { getState: () => state.device } }));
import { capture, captureException, identify, reset, initTelemetry, disableTelemetry, isTelemetryActive } from './telemetry';

const event = { name: 'ui.docs_opened', props: { page: 'encryption' } } as const;
const config = { dsn: 'configured', consent: true };
const permission = { account_id: 'account-a', enabled: true, revision: 'revision-a' };
function boot() { identify('account-a'); initTelemetry(config); }

beforeEach(() => {
  reset(); state.api.mockReset(); state.actor = 'account-a';
  state.device = { accountId: 'account-a', enabled: true, revision: 0 };
  Object.defineProperty(navigator, 'doNotTrack', { value: '0', configurable: true });
  window.history.replaceState(null, '', '/');
  state.api.mockImplementation(async (path: string, options?: { authorityGuard?: () => void }) => {
    options?.authorityGuard?.();
    return path.endsWith('/analytics') ? permission : undefined;
  });
});

describe('authenticated relay analytics', () => {
  it('relays Nyxbot error codes only with active consent and the fresh revision', async () => {
    const failure = { name: 'nyxbot.error', props: {
      error_code: 'NYXBOT_HTTP_FORBIDDEN', engine: 'nyxagent', stage: 'request',
      route: '/assistant/nyxagent/turns', http_status: 403,
    } } as const;
    boot(); window.history.replaceState(null, '', '/assistant');
    await capture(failure);
    expect(state.api.mock.calls[1]![1].body).toEqual({ account_id: 'account-a', revision: 'revision-a', event: failure });
    state.device.enabled = false;
    await capture(failure);
    expect(state.api).toHaveBeenCalledTimes(2);
  });

  it('admits each event with a fresh permission and sends its exact revision', async () => {
    boot();
    await capture(event); await capture(event);
    expect(state.api.mock.calls.map(([path]) => path)).toEqual([
      '/users/me/preferences/analytics', '/users/me/analytics/events',
      '/users/me/preferences/analytics', '/users/me/analytics/events',
    ]);
    expect(state.api.mock.calls[1]![1].body).toEqual({ account_id: 'account-a', revision: 'revision-a', event });
  });

  it('never reuses a cached positive after remote withdrawal', async () => {
    boot(); await capture(event);
    state.api.mockResolvedValue({ ...permission, enabled: false, revision: 'withdrawn' });
    await capture(event);
    expect(state.api.mock.calls.filter(([path]) => path.endsWith('/events'))).toHaveLength(1);
  });

  it('lookup failure and mismatched response actor cannot dispatch', async () => {
    boot(); state.api.mockRejectedValueOnce(new Error('unavailable'));
    await capture(event);
    state.api.mockResolvedValueOnce({ ...permission, account_id: 'account-b' });
    await capture(event);
    expect(state.api.mock.calls.filter(([path]) => path.endsWith('/events'))).toHaveLength(0);
  });

  it('fences a late admission result after account switch', async () => {
    boot();
    let release!: (value: typeof permission) => void;
    state.api.mockImplementationOnce(() => new Promise((resolve) => { release = resolve; }));
    const pending = capture(event);
    await vi.waitFor(() => expect(release).toBeTypeOf('function'));
    state.actor = 'account-b'; state.device = { accountId: 'account-b', enabled: true, revision: 1 };
    identify('account-b');
    release(permission); await pending;
    expect(state.api.mock.calls.filter(([path]) => path.endsWith('/events'))).toHaveLength(0);
  });

  it('fences withdrawal while admission is pending', async () => {
    boot();
    let release!: (value: typeof permission) => void;
    state.api.mockImplementationOnce(() => new Promise((resolve) => { release = resolve; }));
    const pending = capture(event);
    await vi.waitFor(() => expect(release).toBeTypeOf('function'));
    disableTelemetry(); release(permission); await pending;
    expect(state.api.mock.calls).toHaveLength(1);
  });

  it('cross-tab withdrawal and regrant cannot revive an in-flight admission', async () => {
    boot();
    let release!: (value: typeof permission) => void;
    state.api.mockImplementationOnce(() => new Promise((resolve) => { release = resolve; }));
    const pending = capture(event);
    state.device.enabled = false; state.device.revision++;
    state.device.enabled = true; state.device.revision++;
    release(permission); await pending;
    expect(state.api.mock.calls).toHaveLength(1);
  });

  it('bounds concurrent admissions and drops overflow without queueing', async () => {
    boot();
    const releases: Array<(value: typeof permission) => void> = [];
    state.api.mockImplementation(() => new Promise((resolve) => { releases.push(resolve); }));
    const pending = Array.from({ length: 16 }, () => capture(event));
    await vi.waitFor(() => expect(releases).toHaveLength(16));
    await capture(event); expect(state.api).toHaveBeenCalledTimes(16);
    for (const release of releases) release({ ...permission, enabled: false });
    await Promise.all(pending);
  });

  it('requires both local restriction and configured authenticated identity', async () => {
    await capture(event);
    boot(); state.device.enabled = false; await capture(event);
    state.device.enabled = true; state.device.accountId = 'account-b'; await capture(event);
    state.device.accountId = 'account-a'; initTelemetry({ ...config, consent: false }); await capture(event);
    initTelemetry({ ...config, dsn: '' }); await capture(event);
    expect(state.api).not.toHaveBeenCalled();
  });

  it('honors DNT and suppresses authentication/token-bearing pages', async () => {
    boot();
    Object.defineProperty(navigator, 'doNotTrack', { value: '1', configurable: true });
    expect(isTelemetryActive()).toBe(false); await capture(event);
    Object.defineProperty(navigator, 'doNotTrack', { value: '0', configurable: true });
    for (const path of ['/login/agent-key/secret', '/reset-password/secret', '/oauth/callback?code=secret', '/approve/secret']) {
      window.history.replaceState(null, '', path); await capture(event);
    }
    expect(state.api).not.toHaveBeenCalled();
  });

  it('never forwards raw exception text or stacks', async () => {
    boot(); captureException(new Error('distinctive-secret')); await Promise.resolve();
    expect(state.api).not.toHaveBeenCalled();
  });
});
