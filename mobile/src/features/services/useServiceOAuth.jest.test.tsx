import { act, renderHook } from "@testing-library/react-native";
import * as WebBrowser from "expo-web-browser";
import AsyncStorage from "@react-native-async-storage/async-storage";
import { AppState, type AppStateStatus } from "react-native";
import { servicesApi } from "../../lib/api/servicesApi";
import { ApiError } from "../../lib/api/ApiError";
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
      { preferEphemeralSession: true },
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
  expect(result.current.banner).toBeUndefined();
  await unmount();
});

test("success banners use the connection label and can be dismissed", async () => {
  jest.mocked(WebBrowser.openAuthSessionAsync).mockResolvedValue({
    type: "success",
    url: "nyxid://services/oauth-return?status=success",
  });
  const labeledConnection = { ...connection, label: "Production GitHub" };
  const catalog = { ...entry, name: "GitHub catalog entry" };
  jest.mocked(servicesApi.connection).mockResolvedValue(labeledConnection);
  const { result, unmount } = await renderHook(() =>
    useServiceOAuth(true, changed),
  );
  await flush();
  let task!: Promise<void>;
  await act(async () => {
    task = result.current.start(labeledConnection, catalog);
  });
  await act(async () => {
    await jest.advanceTimersByTimeAsync(500);
    await task;
  });
  expect(result.current.banner?.attempt.catalogName).toBe("Production GitHub");
  await act(async () => {
    await result.current.dismiss();
  });
  expect(result.current.banner).toBeUndefined();
  await unmount();
});

test("dismissing a relabeled persisted success clears the stored attempt", async () => {
  await saveServiceOAuth({ ...pending(), catalogName: "Old catalog name" });
  jest.mocked(servicesApi.connection).mockResolvedValue({
    ...connection,
    label: "Current connection name",
    status: "active",
    last_authorized_at: "2026-10-15T06:00:00Z",
  });
  const { result, unmount } = await renderHook(() =>
    useServiceOAuth(true, changed),
  );
  await flush();
  expect(result.current.banner?.attempt.catalogName).toBe(
    "Current connection name",
  );
  expect(result.current.banner?.outcome).toBe("success");
  await act(async () => {
    await result.current.dismiss();
  });
  expect(await loadServiceOAuth()).toBeNull();
  await unmount();
});

test("provider setup errors before browser launch stay actionable and clear the pending attempt", async () => {
  jest.mocked(servicesApi.initiateOAuth).mockRejectedValueOnce(new Error("Configure the OAuth app first"));
  const { result, unmount } = await renderHook(() => useServiceOAuth(true, changed));
  await flush();
  await act(async () => {
    await expect(result.current.start(connection, entry)).rejects.toThrow("Configure the OAuth app first");
  });
  expect(result.current.banner?.outcome).toBe("failed");
  expect(await loadServiceOAuth()).toBeNull();
  expect(WebBrowser.openAuthSessionAsync).not.toHaveBeenCalled();
  await unmount();
});

test("a return-destination 400 retries once without the app parameter", async () => {
  jest.mocked(servicesApi.initiateOAuth)
    .mockRejectedValueOnce(
      new ApiError({
        errorKey: "bad_request",
        errorCode: 1000,
        statusCode: 400,
        message: "invalid app return",
      }),
    )
    .mockResolvedValueOnce({ authorization_url: "https://provider.example/authorize" });
  const { result, unmount } = await renderHook(() => useServiceOAuth(true, changed));
  await flush();
  let task!: Promise<void>;
  await act(async () => {
    task = result.current.start(connection, entry);
  });
  await act(async () => {
    await jest.advanceTimersByTimeAsync(500);
    await task;
  });
  expect(servicesApi.initiateOAuth).toHaveBeenCalledTimes(2);
  expect(jest.mocked(servicesApi.initiateOAuth).mock.calls[1]?.[4]).toEqual({
    appReturnUri: null,
  });
  expect(WebBrowser.openAuthSessionAsync).toHaveBeenCalledWith(
    "https://provider.example/authorize",
    "nyxid://services/oauth-return",
    { preferEphemeralSession: true },
  );
  expect(result.current.banner?.outcome).toBe("pending");
  await unmount();
});

test.each([403, 500])("non-400 initiate status %s is not retried", async (status) => {
  const error = new ApiError({
    errorKey: "request_failed",
    errorCode: 1000,
    statusCode: status,
    message: "server rejected",
  });
  jest.mocked(servicesApi.initiateOAuth).mockRejectedValueOnce(error);
  const { result, unmount } = await renderHook(() => useServiceOAuth(true, changed));
  await flush();
  await act(async () => {
    await expect(result.current.start(connection, entry)).rejects.toBe(error);
  });
  expect(servicesApi.initiateOAuth).toHaveBeenCalledTimes(1);
  await unmount();
});

test("a second 400 is surfaced after the one fallback attempt", async () => {
  const first = new ApiError({
    errorKey: "bad_request",
    errorCode: 1000,
    statusCode: 400,
    message: "first",
  });
  const second = new ApiError({
    errorKey: "bad_request",
    errorCode: 1000,
    statusCode: 400,
    message: "second",
  });
  jest.mocked(servicesApi.initiateOAuth)
    .mockRejectedValueOnce(first)
    .mockRejectedValueOnce(second);
  const { result, unmount } = await renderHook(() => useServiceOAuth(true, changed));
  await flush();
  await act(async () => {
    await expect(result.current.start(connection, entry)).rejects.toBe(second);
  });
  expect(servicesApi.initiateOAuth).toHaveBeenCalledTimes(2);
  expect(WebBrowser.openAuthSessionAsync).not.toHaveBeenCalled();
  await unmount();
});

test("missing OAuth provider is surfaced explicitly", async () => {
  const { result, unmount } = await renderHook(() =>
    useServiceOAuth(true, changed),
  );
  await flush();
  await expect(
    result.current.start(connection, { ...entry, provider_config_id: null }),
  ).rejects.toThrow("This service has no OAuth provider configured.");
  expect(servicesApi.initiateOAuth).not.toHaveBeenCalled();
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
