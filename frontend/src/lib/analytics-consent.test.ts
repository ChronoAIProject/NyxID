import { beforeEach, expect, it, vi } from 'vitest';
const state = vi.hoisted(() => ({ actor: 'account-a', enabled: false, revision: 0, api: vi.fn(), disable: vi.fn(), choose: vi.fn() }));
vi.mock('./api-client', () => ({ apiClient: state.api }));
vi.mock('./telemetry', () => ({ disableTelemetry: state.disable }));
vi.mock('@/stores/auth-store', () => ({ useAuthStore: { getState: () => ({ user: { id: state.actor } }) } }));
vi.mock('@/stores/consent-store', () => ({ useConsentStore: { getState: () => ({ revision: state.revision, setConsent: state.choose }) } }));
import { saveAnalyticsConsent, readAnalyticsConsent } from './analytics-consent';
const result = { account_id: 'account-a', enabled: true, revision: 'rev', changed_at: null };
beforeEach(() => {
  state.actor = 'account-a'; state.enabled = false; state.revision = 0;
  state.api.mockReset(); state.disable.mockReset(); state.choose.mockReset();
  state.choose.mockImplementation((enabled: boolean) => { state.enabled = enabled; state.revision++; });
});
it('binds mutation intent and only enables locally after matching acknowledgement', async () => {
  let release!: (value: typeof result) => void;
  state.api.mockImplementation((_path, options) => {
    options.authorityGuard();
    return new Promise((resolve) => { release = resolve; });
  });
  const saving = saveAnalyticsConsent('account-a', true);
  expect(state.enabled).toBe(false);
  expect(state.api.mock.calls[0]![1].body).toEqual({ account_id: 'account-a', enabled: true });
  release(result); await saving;
  expect(state.enabled).toBe(true);
});
it('does not apply a positive response from a cookie switched to another account', async () => {
  state.api.mockResolvedValue({ ...result, account_id: 'account-b' });
  await expect(saveAnalyticsConsent('account-a', true)).rejects.toThrow('did not match');
  expect(state.enabled).toBe(false);
  await expect(readAnalyticsConsent('account-a')).rejects.toThrow('did not match');
});
it('fences a late positive after local account switch', async () => {
  state.api.mockImplementation(async () => { state.actor = 'account-b'; return result; });
  await expect(saveAnalyticsConsent('account-a', true)).rejects.toThrow('Account changed');
  expect(state.enabled).toBe(false);
});
it('stops this device even if global withdrawal fails, and exposes the failure', async () => {
  state.enabled = true; state.api.mockRejectedValue(new Error('network failure'));
  await expect(saveAnalyticsConsent('account-a', false)).rejects.toThrow('network failure');
  expect(state.disable).toHaveBeenCalledOnce();
  expect(state.enabled).toBe(false);
});

it.each([false, true])('a newer device choice fences a delayed positive response (regrant=%s)', async (regrant) => {
  let release!: (value: typeof result) => void;
  state.api.mockImplementationOnce(() => new Promise((resolve) => { release = resolve; }));
  const saving = saveAnalyticsConsent('account-a', true);
  state.choose(false);
  if (regrant) state.choose(true);
  release(result);
  await expect(saving).rejects.toThrow('Analytics choice changed');
  expect(state.enabled).toBe(regrant);
});

it('rejects an acknowledgement that contradicts withdrawal', async () => {
  state.api.mockResolvedValue(result);
  await expect(saveAnalyticsConsent('account-a', false)).rejects.toThrow('did not match');
  expect(state.enabled).toBe(false);
});
