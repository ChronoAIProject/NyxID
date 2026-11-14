import { act, renderHook } from "@testing-library/react-native";
import * as WebBrowser from "expo-web-browser";
import AsyncStorage from "@react-native-async-storage/async-storage";
import { AppState, type AppStateStatus } from "react-native";
import { servicesApi } from "../../lib/api/servicesApi";
import { setAuthAccountId } from "../../lib/auth/authAccount";
import { useServiceOAuth } from "./useServiceOAuth";
import {
  clearServiceOAuth,
  recordServiceOAuthReturn,
  loadServiceOAuth,
  saveServiceOAuth,
  SERVICE_OAUTH_STORAGE_KEY,
} from "./serviceOAuthStore";
import {
  SERVICE_OAUTH_TTL_MS,
  type PendingServiceOAuth,
} from "./serviceOAuthModel";

jest.mock("@react-native-async-storage/async-storage", () =>
  require("@react-native-async-storage/async-storage/jest/async-storage-mock"),
);
jest.mock("expo-web-browser", () => ({
  WebBrowserResultType: { CANCEL: "cancel", DISMISS: "dismiss" },
  openAuthSessionAsync: jest.fn(),
}));
jest.mock("../../lib/env", () => ({ NYXID_APP_SCHEME: "nyxid" }));
jest.mock("../../lib/api/servicesApi", () => ({
  servicesApi: {
    connection: jest.fn(),
    catalogEntry: jest.fn(),
    initiateOAuth: jest.fn(),
  },
}));
const connection = {
  id: "connection",
  label: "GitHub",
  slug: "github",
  status: "pending_auth",
  is_active: true,
  auto_connected: false,
  credential_type: "oauth2",
};
const entry = {
  slug: "github",
  name: "GitHub",
  provider_config_id: "provider",
  auth_method: "bearer",
  auth_key_name: "Authorization",
  service_type: "http",
  requires_credential: true,
  requires_gateway_url: false,
};
const changed = jest.fn(async () => {});
const pending = (): PendingServiceOAuth => ({
  accountId: "one",
  connectionId: "connection",
  providerId: "provider",
  catalogSlug: "github",
  catalogName: "GitHub",
  baselineStatus: "pending_auth",
  baselineAuthorizedAt: null,
  startedAt: Date.now(),
  expiresAt: Date.now() + SERVICE_OAUTH_TTL_MS,
});
async function flush() {
  await act(async () => {
    await Promise.resolve();
    await Promise.resolve();
  });
}

beforeEach(async () => {
  jest.useFakeTimers();
  jest
    .spyOn(AppState, "addEventListener")
    .mockReturnValue({ remove: jest.fn() });
  setAuthAccountId(null);
  await AsyncStorage.clear();
  setAuthAccountId("one");
  jest.mocked(servicesApi.initiateOAuth).mockResolvedValue({
    authorization_url: "https://provider.example/authorize",
  });
  jest.mocked(servicesApi.connection).mockResolvedValue(connection);
  jest.mocked(servicesApi.catalogEntry).mockResolvedValue(entry);
  jest
    .mocked(WebBrowser.openAuthSessionAsync)
    .mockResolvedValue({ type: WebBrowser.WebBrowserResultType.CANCEL });
});
afterEach(() => {
  jest.useRealTimers();
  jest.restoreAllMocks();
  jest.clearAllMocks();
});

test.each(["cancel", "dismiss"] as const)(
  "old backend without a return: %s never means success; two bounded reads and Check again",
  async (type) => {
    jest.mocked(WebBrowser.openAuthSessionAsync).mockResolvedValue({
      type:
        type === "cancel"
          ? WebBrowser.WebBrowserResultType.CANCEL
          : WebBrowser.WebBrowserResultType.DISMISS,
    });
    const { result, unmount } = await renderHook(() =>
      useServiceOAuth(true, changed),
    );
    await flush();
    let task!: Promise<void>;
    await act(async () => {
      task = result.current.start(connection, entry);
    });
    await act(async () => {
      await jest.advanceTimersByTimeAsync(500);
      await task;
    });
    expect(result.current.banner?.outcome).toBe("pending");
    expect(servicesApi.connection).toHaveBeenCalledTimes(2);
    expect(await loadServiceOAuth()).toMatchObject({
      accountId: "one",
      connectionId: "connection",
    });
    expect(WebBrowser.openAuthSessionAsync).toHaveBeenCalledWith(
      "https://provider.example/authorize",
      "nyxid://services/oauth-return",
    );
    await unmount();
  },
);

test("success URL still needs a changed active server connection", async () => {
  jest.mocked(WebBrowser.openAuthSessionAsync).mockResolvedValue({
    type: "success",
    url: "nyxid://services/oauth-return?status=success",
  });
  const { result, unmount } = await renderHook(() =>
    useServiceOAuth(true, changed),
  );
  await flush();
  let task!: Promise<void>;
  await act(async () => {
    task = result.current.start(connection, entry);
  });
  await act(async () => {
    await jest.advanceTimersByTimeAsync(500);
    await task;
  });
  expect(result.current.banner?.outcome).toBe("unconfirmed");
  jest
    .mocked(servicesApi.connection)
    .mockResolvedValue({ ...connection, status: "active" });
  await act(async () => {
    await result.current.check();
  });
  expect(result.current.banner?.outcome).toBe("success");
  expect(await loadServiceOAuth()).toBeNull();
  await act(async () => {
    recordServiceOAuthReturn("success");
  });
  await flush();
  expect(result.current.banner?.outcome).toBe("success");
  await unmount();
});

test("relaunch recovers a persisted attempt; foreground refresh resolves it", async () => {
  await saveServiceOAuth(pending());
  const handlers: ((state: AppStateStatus) => void)[] = [];
  const listener = jest
    .spyOn(AppState, "addEventListener")
    .mockImplementation((_name, callback) => {
      handlers.push(callback);
      return { remove: jest.fn() };
    });
  const { result, unmount } = await renderHook(() =>
    useServiceOAuth(true, changed),
  );
  await flush();
  expect(result.current.banner?.outcome).toBe("pending");
  jest
    .mocked(servicesApi.connection)
    .mockResolvedValue({ ...connection, status: "active" });
  await act(async () => {
    handlers[0]?.("active");
  });
  await flush();
  expect(result.current.banner?.outcome).toBe("success");
  expect(await loadServiceOAuth()).toBeNull();
  await unmount();
  listener.mockRestore();
});

test("logout, account switch and expiry discard persisted attempts", async () => {
  for (const next of [null, "two"]) {
    setAuthAccountId("one");
    await saveServiceOAuth(pending());
    setAuthAccountId(next);
    await loadServiceOAuth();
    expect(await AsyncStorage.getItem(SERVICE_OAUTH_STORAGE_KEY)).toBeNull();
  }
  setAuthAccountId("one");
  await saveServiceOAuth(pending());
  await jest.advanceTimersByTimeAsync(SERVICE_OAUTH_TTL_MS);
  expect(await loadServiceOAuth()).toBeNull();
});

test("a late terminal outcome or expiry cannot clear a newer attempt for the same account", async () => {
  const old = pending();
  await saveServiceOAuth(old);
  const newer = { ...old, connectionId: "new-connection" };
  await saveServiceOAuth(newer);
  await clearServiceOAuth(old);
  expect(await loadServiceOAuth()).toEqual(newer);
});

test("a return during an in-flight recovery read schedules a fresh check", async () => {
  await saveServiceOAuth(pending());
  let resolve!: (value: typeof connection) => void;
  jest.mocked(servicesApi.connection).mockImplementationOnce(
    () =>
      new Promise((done) => {
        resolve = done;
      }),
  );
  const { result, unmount } = await renderHook(() =>
    useServiceOAuth(true, changed),
  );
  await flush();
  // Starting again replaces the baseline while the old server read is pending.
  await act(async () => {
    await result.current.start(connection, entry);
  });
  jest
    .mocked(servicesApi.connection)
    .mockResolvedValue({ ...connection, status: "active" });
  await act(async () => {
    resolve(connection);
  });
  await flush();
  expect(servicesApi.connection).toHaveBeenCalledTimes(2);
  expect(result.current.banner?.outcome).toBe("success");
  expect(await loadServiceOAuth()).toBeNull();
  await unmount();
});
