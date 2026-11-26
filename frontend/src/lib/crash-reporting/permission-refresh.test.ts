import { QueryClient } from "@tanstack/react-query";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import type { User } from "@/types/api";

const mocks = vi.hoisted(() => ({ read: vi.fn(), api: vi.fn() }));
vi.mock("../analytics-consent", () => ({ readAnalyticsConsent: mocks.read }));
vi.mock("../api-client", () => ({ apiClient: mocks.api }));
vi.mock("../telemetry", () => ({
  isDntActive: () => false,
  isSensitivePath: () => false,
}));
vi.mock("@/stores/auth-store", async () => {
  const { create } = await import("zustand");
  return {
    useAuthStore: create(() => ({
      user: { id: "account-a" },
      isLoading: false,
      mfaRequired: false,
    })),
  };
});
vi.mock("@/stores/consent-store", async () => {
  const { create } = await import("zustand");
  return {
    useConsentStore: create(() => ({
      accountId: "account-a", enabled: true, policy: 2, revision: 0,
      asked: true, persistenceError: false,
    })),
  };
});

const permission = {
  enabled: true, policy_version: "2", diagnostics_status: "available",
  diagnostics_available: true, revision: "server-revision",
};
let client: QueryClient;
let stop: (() => void) | undefined;
let auth: typeof import("@/stores/auth-store").useAuthStore;
let consent: typeof import("@/stores/consent-store").useConsentStore;
let reporter: typeof import("./reporter");
let subscribe: typeof import("./permission-refresh").subscribeCrashPermission;

beforeEach(async () => {
  vi.resetModules();
  mocks.read.mockReset().mockImplementation(async (actor: string) => ({
    ...permission, account_id: actor,
  }));
  mocks.api.mockReset().mockResolvedValue({});
  auth = (await import("@/stores/auth-store")).useAuthStore;
  consent = (await import("@/stores/consent-store")).useConsentStore;
  auth.setState({ user: { id: "account-a" } as User, isLoading: false });
  consent.setState({
    accountId: "account-a", enabled: true, policy: 2, revision: 0,
    asked: true, persistenceError: false,
  });
  reporter = await import("./reporter");
  subscribe = (await import("./permission-refresh")).subscribeCrashPermission;
  client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  window.history.replaceState(null, "", "/dashboard");
});
afterEach(() => {
  stop?.();
  stop = undefined;
  client.clear();
});

function boot(offered?: boolean) {
  client.setQueryData(["public-config"], {
    ...(offered === undefined ? {} : { crash_diagnostics_offered: offered }),
  });
  stop = subscribe(client);
}
async function flushed() {
  for (let i = 0; i < 5; i++) await Promise.resolve();
}
function failure() {
  const error = new TypeError("not retained");
  error.stack = "TypeError\n at fn (http://localhost:3000/assets/app.js:12:4)";
  return error;
}
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}

it("an opted-in page boot with an absent public hint makes zero reporter preference GETs", async () => {
  boot();
  await flushed();
  await reporter.refreshCrashPermission();
  await reporter.reportCrash(failure());
  expect(mocks.read).not.toHaveBeenCalled();
  expect(mocks.api).not.toHaveBeenCalled();
});

it("an offered page boot makes one discovery GET and each report still checks fresh consent", async () => {
  boot(true);
  await flushed();
  expect(mocks.read).toHaveBeenCalledExactlyOnceWith("account-a");
  await reporter.reportCrash(failure());
  expect(mocks.read).toHaveBeenCalledTimes(2);
  expect(mocks.api).toHaveBeenCalledOnce();
});

it("unrelated auth consent and query-cache updates make no extra preference GETs", async () => {
  boot(true);
  await flushed();
  auth.setState({ isLoading: true });
  auth.setState({
    user: { ...auth.getState().user!, display_name: "updated profile" },
    isLoading: false, mfaRequired: true,
  });
  consent.setState({ asked: false, persistenceError: true });
  client.setQueryData(["unrelated"], { updated: true });
  client.setQueryData(["public-config"], {
    crash_diagnostics_offered: true, telemetry_share_analytics: true,
  });
  await flushed();
  expect(mocks.read).toHaveBeenCalledOnce();
});

it("concurrent refresh triggers share one in-flight preference read", async () => {
  const response = deferred<typeof permission>();
  mocks.read.mockReturnValueOnce(response.promise);
  boot(true);
  const first = reporter.refreshCrashPermission();
  const second = reporter.refreshCrashPermission();
  expect(second).toBe(first);
  await flushed();
  expect(mocks.read).toHaveBeenCalledOnce();
  response.resolve(permission);
  await Promise.all([first, second]);
  expect(mocks.read).toHaveBeenCalledOnce();
});

it("an actor change and a device revision change each start one new preference read", async () => {
  boot(true);
  await flushed();
  auth.setState({ user: { id: "account-b" } as User });
  consent.setState({ accountId: "account-b", revision: 1 });
  await flushed();
  expect(mocks.read.mock.calls.map(([actor]) => actor)).toEqual([
    "account-a", "account-b",
  ]);
  consent.setState({ revision: 2 });
  await flushed();
  expect(mocks.read.mock.calls.map(([actor]) => actor)).toEqual([
    "account-a", "account-b", "account-b",
  ]);
});

it("cached public hint changes re-evaluate discovery and absence fails closed", async () => {
  boot();
  await flushed();
  client.setQueryData(["public-config"], { crash_diagnostics_offered: true });
  await flushed();
  expect(mocks.read).toHaveBeenCalledOnce();
  client.setQueryData(["public-config"], {});
  await reporter.reportCrash(failure());
  expect(mocks.read).toHaveBeenCalledOnce();
  expect(mocks.api).not.toHaveBeenCalled();
  client.setQueryData(["public-config"], { crash_diagnostics_offered: true });
  await flushed();
  expect(mocks.read).toHaveBeenCalledTimes(2);
});

it("a stale refresh neither arms capture nor clears a successor identity's in-flight read", async () => {
  const first = deferred<typeof permission>();
  const second = deferred<typeof permission>();
  mocks.read.mockReturnValueOnce(first.promise).mockReturnValueOnce(second.promise);
  boot(true);
  await flushed();
  consent.setState({ revision: 1 });
  const successor = reporter.refreshCrashPermission();
  await flushed();
  expect(mocks.read).toHaveBeenCalledTimes(2);
  first.resolve(permission);
  await flushed();
  await reporter.reportCrash(failure());
  expect(mocks.api).not.toHaveBeenCalled();
  expect(reporter.refreshCrashPermission()).toBe(successor);
  second.resolve(permission);
  await successor;
  await reporter.reportCrash(failure());
  expect(mocks.read).toHaveBeenCalledTimes(3);
  expect(mocks.api).toHaveBeenCalledOnce();
});

it("public hint withdrawal fences an in-flight available permission response", async () => {
  const response = deferred<typeof permission>();
  mocks.read.mockReturnValueOnce(response.promise);
  boot(true);
  await flushed();
  client.setQueryData(["public-config"], {});
  response.resolve(permission);
  await flushed();
  await reporter.reportCrash(failure());
  expect(mocks.read).toHaveBeenCalledOnce();
  expect(mocks.api).not.toHaveBeenCalled();
});

it.each(["enabled", "policy", "accountId"] as const)(
  "a device %s change invalidates capture and re-enabling starts one new read",
  async (field) => {
    boot(true);
    await flushed();
    const device = consent.getState();
    if (field === "enabled") consent.setState({ enabled: false });
    else if (field === "policy") consent.setState({ policy: 1 });
    else consent.setState({ accountId: "another-account" });
    await reporter.reportCrash(failure());
    expect(mocks.read).toHaveBeenCalledOnce();
    expect(mocks.api).not.toHaveBeenCalled();
    consent.setState({
      enabled: device.enabled, policy: device.policy, accountId: device.accountId,
    });
    await flushed();
    expect(mocks.read).toHaveBeenCalledTimes(2);
  },
);
