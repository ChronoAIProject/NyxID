import { beforeEach, expect, it, vi } from 'vitest';
const state = vi.hoisted(() => ({ actor: 'account-a', enabled: false, asked: false, revision: 0, api: vi.fn(), disable: vi.fn(), choose: vi.fn() }));
vi.mock('./api-client', () => ({ apiClient: state.api }));
vi.mock('./telemetry', () => ({ disableTelemetry: state.disable }));
vi.mock('@/stores/auth-store', () => ({ useAuthStore: { getState: () => ({ user: { id: state.actor } }) } }));
vi.mock('@/stores/consent-store', () => ({ useConsentStore: { getState: () => ({ enabled:state.enabled, accountId:state.actor, revision: state.revision, setConsent: state.choose }) } }));
import { saveAnalyticsConsent, readAnalyticsConsent } from './analytics-consent';
const result = { account_id: 'account-a', enabled: true, revision: 'rev', changed_at: null };
beforeEach(() => {
  state.actor = 'account-a'; state.enabled = false; state.asked = false; state.revision = 0;
  state.api.mockReset(); state.disable.mockReset(); state.choose.mockReset();
  state.choose.mockImplementation((enabled: boolean, _policy?: 1|2, options?: { markAsked?: boolean }) => {
    state.enabled = enabled;
    if (options?.markAsked !== false) state.asked = true;
    state.revision++;
  });
});
it('binds mutation intent and only enables locally after matching acknowledgement', async () => {
  let release!: (value: typeof result) => void;
  state.api.mockImplementation((_path, options) => {
    options.authorityGuard();
    return new Promise((resolve) => { release = resolve; });
  });
  const saving = saveAnalyticsConsent('account-a', true);
  expect(state.enabled).toBe(false);
  expect(state.asked).toBe(false);
  expect(state.api.mock.calls[0]![1].body).toEqual({ account_id: 'account-a', enabled: true });
  release(result); await saving;
  expect(state.enabled).toBe(true);
  expect(state.asked).toBe(true);
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
  expect(state.asked).toBe(false);
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

it.each(['1', '2', undefined])('negotiates policy 2 against advertised policy %s without changing legacy requests', async (current) => {
  state.api.mockResolvedValueOnce({ ...result, current_policy_version: current }).mockResolvedValueOnce(result);
  await saveAnalyticsConsent('account-a', true, 2);
  expect(state.api.mock.calls[1]![1].body).toEqual({ account_id: 'account-a', enabled: true, ...(current === '2' ? { policy_version: '2' } : {}) });
  expect(state.choose).toHaveBeenLastCalledWith(true, current === '2' ? 2 : 1);
});

it('a newer device withdrawal fences the policy negotiation before any PUT', async () => {
  let release!: (value: typeof result & { current_policy_version: string }) => void;
  state.api.mockImplementationOnce(() => new Promise(resolve => { release = resolve; }));
  const pending = saveAnalyticsConsent('account-a', true, 2);
  state.choose(false);
  release({ ...result, current_policy_version: '2' });
  await expect(pending).rejects.toThrow('Analytics choice changed');
  expect(state.api).toHaveBeenCalledOnce();
  expect(state.enabled).toBe(false);
});

it('withdrawal stops immediately even when policy negotiation fails', async () => {
  state.enabled = true;
  state.api.mockRejectedValueOnce(new Error('offline'));
  const pending = saveAnalyticsConsent('account-a', false, 2);
  expect(state.enabled).toBe(false);
  expect(state.disable).toHaveBeenCalledOnce();
  await expect(pending).rejects.toThrow('offline');
});

it.each([true,false])('failed updated acknowledgement preserves the prior device choice enabled=%s',async(enabled)=>{
  state.enabled=enabled; state.api.mockRejectedValue(new Error('offline'));
  await expect(saveAnalyticsConsent('account-a',true,2)).rejects.toThrow('offline');
  expect(state.enabled).toBe(enabled);
  if(enabled) expect(state.disable).not.toHaveBeenCalled();
});
