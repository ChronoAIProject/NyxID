import * as Notifications from "expo-notifications";
import * as SecureStore from "expo-secure-store";
import { AppState, type AppStateStatus } from "react-native";
import { mobileApi } from "../api/mobileApi";
import { ApiError } from "../api/ApiError";
import { setPushDeliverySuppressed } from "./pushDeliveryState";
import { getAuthAccountId, setAuthAccountId } from "../auth/authAccount";
import {
  activatePushAfterLogin,
  capturePushLogout,
  deactivatePushOnLogout,
  isPushDeliverySuppressed,
  initializeNotificationRuntime,
} from "./pushNotifications";
const mockStore = new Map<string, string>();
let mockRefresh: (token: unknown) => void;
let notificationListener: ((notification: any) => void) | undefined;
let appStateListener: ((state: AppStateStatus) => void) | undefined;
let mockHandler: Notifications.NotificationHandler | null;
jest.mock("../env", () => ({
  NYXID_APP_SCHEME: "nyxid",
  NYXID_TRUSTED_FRONTEND_URLS: [],
  NYXID_TRUSTED_API_BASE_URLS: [],
}));
jest.mock("../api/mobileApi", () => ({
  mobileApi: {
    unregisterPushToken: jest.fn(),
    registerPushToken: jest.fn(),
    rotatePushToken: jest.fn(),
  },
}));
jest.mock("expo-secure-store", () => ({
  getItemAsync: jest.fn(async (key) => mockStore.get(key) ?? null),
  setItemAsync: jest.fn(async (key, value) => {
    mockStore.set(key, value);
  }),
  deleteItemAsync: jest.fn(async (key) => {
    mockStore.delete(key);
  }),
}));
jest.mock("expo-task-manager", () => ({
  isTaskDefined: () => true,
  isTaskRegisteredAsync: async () => true,
}));
jest.mock("expo-notifications", () => {
  const getDevicePushTokenAsync = jest.fn(async () => ({
    type: "ios",
    data: "same-native-token",
  }));
  return {
    setNotificationHandler: jest.fn((handler) => {
      mockHandler = handler;
    }),
    addNotificationReceivedListener: jest.fn(
      (listener: (notification: any) => void) => {
        notificationListener = listener;
        return { remove() {} };
      },
    ),
    addPushTokenListener: (listener: typeof mockRefresh) => {
      mockRefresh = listener;
      return { remove() {} };
    },
    unregisterForNotificationsAsync: jest.fn(async () => {}),
    dismissAllNotificationsAsync: jest.fn(async () => {}),
    setBadgeCountAsync: jest.fn(async () => true),
    clearLastNotificationResponseAsync: jest.fn(async () => {}),
    getPermissionsAsync: async () => ({ granted: true, status: "granted" }),
    getDevicePushTokenAsync,
  };
});
beforeEach(() => {
  AppState.currentState = "active";
  mockStore.clear();
  setPushDeliverySuppressed(false);
  jest.mocked(SecureStore.setItemAsync).mockImplementation(async (key, value) => {
    mockStore.set(key, value);
  });
  jest.mocked(SecureStore.deleteItemAsync).mockImplementation(async (key) => {
    mockStore.delete(key);
  });
  jest
    .mocked(Notifications.unregisterForNotificationsAsync)
    .mockReset()
    .mockResolvedValue();
  jest
    .mocked(Notifications.getDevicePushTokenAsync)
    .mockReset()
    .mockResolvedValue({ type: "ios", data: "same-native-token" } as never);
  setAuthAccountId("one");
  appStateListener = undefined;
  jest.spyOn(AppState, "addEventListener").mockImplementation((_event, listener) => {
    appStateListener = listener;
    return { remove: jest.fn() };
  });
  jest.clearAllMocks();
  jest
    .mocked(mobileApi.unregisterPushToken)
    .mockResolvedValue({ deleted: true } as never);
});
afterEach(() => {
  jest.restoreAllMocks();
  jest.useRealTimers();
  setAuthAccountId(null);
});

test("runtime installs its handler and listeners while the push queue is blocked", async () => {
  jest.useFakeTimers();
  let release!: () => void;
  let started!: () => void;
  const registering = new Promise<void>((resolve) => {
    started = resolve;
  });
  jest.mocked(mobileApi.registerPushToken).mockImplementationOnce(
    () => new Promise((resolve) => {
      release = () => resolve({ registered: true } as never);
      started();
    }),
  );
  const registration = activatePushAfterLogin({ forceRegister: true });
  await registering;
  const stop = await initializeNotificationRuntime();
  expect(Notifications.setNotificationHandler).toHaveBeenCalledTimes(1);
  expect(Notifications.addNotificationReceivedListener).toHaveBeenCalledTimes(1);
  expect(appStateListener).toBeDefined();
  release();
  await registration;
  await deactivatePushOnLogout();
  stop();
});

test("native token acquisition times out and frees the push queue", async () => {
  jest.useFakeTimers();
  const warning = jest.spyOn(console, "warn").mockImplementation(() => {});
  jest.mocked(Notifications.getDevicePushTokenAsync)
    .mockImplementationOnce(() => new Promise(() => {}));
  const registration = activatePushAfterLogin({ forceRegister: true });
  await jest.advanceTimersByTimeAsync(15_000);
  await expect(registration).resolves.toMatchObject({
    registered: false, reason: "token_unavailable",
  });
  expect(warning).toHaveBeenCalledWith(
    "[push] native push token unavailable", expect.any(Error),
  );
  await expect(activatePushAfterLogin({ forceRegister: true })).resolves
    .toMatchObject({ registered: true });
  expect(Notifications.getDevicePushTokenAsync).toHaveBeenCalledTimes(2);
  await deactivatePushOnLogout();
});

test("logout fences a late A registration and B rebinds its token before native registration", async () => {
  jest.useFakeTimers();
  let release!: () => void;
  let started!: () => void;
  const registering = new Promise<void>((resolve) => {
    started = resolve;
  });
  jest.mocked(mobileApi.registerPushToken).mockImplementationOnce(
    () => new Promise((resolve) => {
      expect(getAuthAccountId()).toBe("one");
      release = () => resolve({ registered: true } as never);
      started();
    }),
  );
  const registration = activatePushAfterLogin({ forceRegister: true });
  await registering;
  setAuthAccountId(null);
  const logout = deactivatePushOnLogout();
  await jest.advanceTimersByTimeAsync(10_000);
  await expect(logout).resolves.toBe(false);
  expect(mobileApi.unregisterPushToken).not.toHaveBeenCalled();
  expect(Notifications.unregisterForNotificationsAsync).toHaveBeenCalledTimes(1);
  expect(JSON.parse(mockStore.get("nyxid.push.pending_unbind")!)).toMatchObject({
    token: "same-native-token", nativeUnregisterPending: false,
  });
  expect(isPushDeliverySuppressed()).toBe(true);
  expect(Notifications.dismissAllNotificationsAsync).toHaveBeenCalledTimes(1);
  expect(Notifications.setBadgeCountAsync).toHaveBeenCalledWith(0);
  expect(
    Notifications.clearLastNotificationResponseAsync,
  ).toHaveBeenCalledTimes(1);
  jest.mocked(Notifications.getDevicePushTokenAsync).mockClear();
  release();
  await expect(registration).resolves.toMatchObject({ registered: false });
  expect(mockStore.has("nyxid.push.device_token")).toBe(false);
  expect(mockStore.has("nyxid.push.pending_unbind")).toBe(true);
  expect(isPushDeliverySuppressed()).toBe(true);
  expect(Notifications.getDevicePushTokenAsync).not.toHaveBeenCalled();
  expect(SecureStore.setItemAsync).not.toHaveBeenCalledWith(
    "nyxid.push.device_token", expect.any(String),
  );
  setAuthAccountId("two");
  jest.mocked(mobileApi.registerPushToken).mockImplementationOnce(async (body) => {
    expect(getAuthAccountId()).toBe("two");
    expect(body.token).toBe("same-native-token");
    expect(Notifications.getDevicePushTokenAsync).not.toHaveBeenCalled();
    expect(mockStore.has("nyxid.push.pending_unbind")).toBe(true);
    expect(isPushDeliverySuppressed()).toBe(true);
    return { registered: true } as never;
  });
  await activatePushAfterLogin();
  expect(mobileApi.registerPushToken).toHaveBeenCalledTimes(2);
  expect(Notifications.getDevicePushTokenAsync).toHaveBeenCalledTimes(1);
  expect(mockStore.has("nyxid.push.pending_unbind")).toBe(false);
  expect(isPushDeliverySuppressed()).toBe(false);
  expect(
    jest.mocked(mobileApi.registerPushToken).mock.invocationCallOrder[1],
  ).toBeLessThan(
    jest.mocked(Notifications.getDevicePushTokenAsync).mock.invocationCallOrder[0]!,
  );
  await deactivatePushOnLogout();
});

test("logout without a known token still unregisters natively after the queue budget", async () => {
  jest.useFakeTimers();
  let release!: (value: never) => void;
  let started!: () => void;
  const acquiring = new Promise<void>((resolve) => {
    started = resolve;
  });
  jest.mocked(Notifications.getDevicePushTokenAsync).mockImplementationOnce(
    () => new Promise((resolve) => {
      release = resolve;
      started();
    }),
  );
  const registration = activatePushAfterLogin();
  await acquiring;
  const logout = deactivatePushOnLogout();
  await jest.advanceTimersByTimeAsync(10_000);
  await expect(logout).resolves.toBe(false);
  expect(Notifications.unregisterForNotificationsAsync).toHaveBeenCalledTimes(1);
  expect(mockStore.has("nyxid.push.pending_unbind")).toBe(false);
  release({ type: "ios", data: "late-token" } as never);
  await registration;
  expect(mobileApi.registerPushToken).not.toHaveBeenCalled();
});

test("hung native unregister attempts time out three times and preserve recovery", async () => {
  jest.useFakeTimers();
  const warning = jest.spyOn(console, "warn").mockImplementation(() => {});
  mockStore.set("nyxid.push.device_token", "same-native-token");
  jest.mocked(mobileApi.unregisterPushToken)
    .mockRejectedValueOnce(new Error("offline"));
  jest.mocked(Notifications.unregisterForNotificationsAsync)
    .mockImplementation(() => new Promise(() => {}));
  const logout = deactivatePushOnLogout();
  await jest.advanceTimersByTimeAsync(6_300);
  await expect(logout).resolves.toBe(false);
  expect(Notifications.unregisterForNotificationsAsync).toHaveBeenCalledTimes(3);
  expect(JSON.parse(mockStore.get("nyxid.push.pending_unbind")!))
    .toMatchObject({ nativeUnregisterPending: true });
  expect(isPushDeliverySuppressed()).toBe(true);
  expect(warning).toHaveBeenCalledWith("[push] native unregister unavailable");
});

test("failed server unbind invalidates native delivery and always dismisses tray and badge", async () => {
  mockStore.set("nyxid.push.device_token", "same-native-token");
  jest
    .mocked(mobileApi.unregisterPushToken)
    .mockRejectedValueOnce(new Error("offline"));
  expect(await deactivatePushOnLogout()).toBe(false);
  expect(Notifications.unregisterForNotificationsAsync).toHaveBeenCalledTimes(
    1,
  );
  expect(Notifications.dismissAllNotificationsAsync).toHaveBeenCalledTimes(1);
  expect(Notifications.setBadgeCountAsync).toHaveBeenCalledWith(0);
  expect(
    Notifications.clearLastNotificationResponseAsync,
  ).toHaveBeenCalledTimes(1);
  expect(mockStore.has("nyxid.push.device_token")).toBe(false);
  expect(JSON.parse(mockStore.get("nyxid.push.pending_unbind")!)).toEqual(
    expect.objectContaining({
      token: "same-native-token",
      provider: "apns",
      platform: "ios",
    }),
  );
  setAuthAccountId("two");
  await activatePushAfterLogin();
  expect(mobileApi.registerPushToken).toHaveBeenCalledWith(
    expect.objectContaining({ token: "same-native-token" }),
  );
});
test("every logout clears tray and badge, including no token and successful unbind", async () => {
  for (const token of [null, "native-token"]) {
    if (token) mockStore.set("nyxid.push.device_token", token);
    expect(await deactivatePushOnLogout()).toBe(true);
  }
  expect(Notifications.dismissAllNotificationsAsync).toHaveBeenCalledTimes(2);
  expect(Notifications.setBadgeCountAsync).toHaveBeenCalledTimes(2);
  expect(Notifications.unregisterForNotificationsAsync).not.toHaveBeenCalled();
});
test("native invalidation failure is guarded and still clears cached registration", async () => {
  mockStore.set("nyxid.push.device_token", "native-token");
  const warning = jest.spyOn(console, "warn").mockImplementation(() => {});
  jest
    .mocked(mobileApi.unregisterPushToken)
    .mockRejectedValueOnce(new Error("offline"));
  jest
    .mocked(Notifications.unregisterForNotificationsAsync)
    .mockRejectedValue(new Error("unsupported runtime"));
  await expect(deactivatePushOnLogout()).resolves.toBe(false);
  expect(Notifications.unregisterForNotificationsAsync).toHaveBeenCalledTimes(3);
  expect(warning).toHaveBeenCalledWith("[push] native unregister unavailable");
  warning.mockRestore();
  expect(mockStore.has("nyxid.push.device_token")).toBe(false);
});

test("logout waits for an in-flight registration before sending the authenticated unbind", async () => {
  let settleRegistration!: () => void;
  let started!: () => void;
  const registering = new Promise<void>((resolve) => {
    started = resolve;
  });
  jest.mocked(mobileApi.registerPushToken).mockImplementationOnce(
    () => new Promise((resolve) => {
      settleRegistration = () => resolve({ registered: true } as never);
      started();
    }),
  );
  mockStore.set("nyxid.push.device_token", "same-native-token");
  const registration = activatePushAfterLogin({ forceRegister: true });
  await registering;
  const logout = deactivatePushOnLogout();
  expect(mobileApi.unregisterPushToken).not.toHaveBeenCalled();
  expect(settleRegistration).toBeDefined();
  settleRegistration();
  await registration;
  await logout;
  expect(mobileApi.registerPushToken).toHaveBeenCalledTimes(1);
  expect(mobileApi.unregisterPushToken).toHaveBeenCalledTimes(1);
  expect(
    jest.mocked(mobileApi.registerPushToken).mock.invocationCallOrder[0],
  ).toBeLessThan(
    jest.mocked(mobileApi.unregisterPushToken).mock.invocationCallOrder[0]!,
  );
});

test("logout keeps an unresolved first-registration token for a failed unbind", async () => {
  const warning = jest.spyOn(console, "warn").mockImplementation(() => {});
  jest
    .mocked(mobileApi.registerPushToken)
    .mockRejectedValueOnce(new Error("registration outcome unknown"));
  jest
    .mocked(mobileApi.unregisterPushToken)
    .mockRejectedValueOnce(new Error("offline"));
  await activatePushAfterLogin({ forceRegister: true });
  expect(mobileApi.registerPushToken).toHaveBeenCalledWith(
    expect.objectContaining({ token: "same-native-token" }),
  );
  await expect(deactivatePushOnLogout()).resolves.toBe(false);
  expect(mobileApi.unregisterPushToken).toHaveBeenCalledWith(
    expect.objectContaining({ token: "same-native-token" }),
  );
  expect(mockStore.has("nyxid.push.pending_unbind")).toBe(true);
  expect(warning).toHaveBeenCalledWith(
    "[push] register token failed",
    expect.any(Error),
  );
  warning.mockRestore();
});

test("pending unbind suppresses notification presentation until a later registration succeeds", async () => {
  mockStore.set("nyxid.push.device_token", "same-native-token");
  jest.mocked(mobileApi.unregisterPushToken).mockRejectedValueOnce(new Error("offline"));
  await deactivatePushOnLogout();
  const stop = await initializeNotificationRuntime();
  const handler = mockHandler;
  expect(handler).toBeDefined();
  const behavior = await handler!.handleNotification({
    request: {
      identifier: "approval-a",
      content: { title: "Approval", body: "Decide", data: { type: "approval_request", request_id: "a", challenge_id: "a" } },
    },
  } as never);
  expect(behavior.shouldShowAlert).toBe(false);
  notificationListener?.({
    request: {
      content: { title: "Approval", body: "Decide", data: { type: "approval_request", request_id: "a", challenge_id: "a" } },
    },
  });
  setAuthAccountId("two");
  await activatePushAfterLogin();
  expect(mockStore.has("nyxid.push.pending_unbind")).toBe(false);
  expect(isPushDeliverySuppressed()).toBe(false);
  stop();
});

test("pending rebind succeeds before same-token native registration and does not rotate", async () => {
  setPushDeliverySuppressed(true);
  mockStore.set(
    "nyxid.push.pending_unbind",
    JSON.stringify({
      token: "same-native-token",
      provider: "apns",
      platform: "ios",
      createdAt: new Date().toISOString(),
    }),
  );
  await activatePushAfterLogin();
  expect(mobileApi.registerPushToken).toHaveBeenCalledTimes(1);
  expect(mobileApi.registerPushToken).toHaveBeenCalledWith(
    expect.objectContaining({ token: "same-native-token" }),
  );
  expect(mobileApi.rotatePushToken).not.toHaveBeenCalled();
  expect(Notifications.getDevicePushTokenAsync).toHaveBeenCalledTimes(1);
  expect(mockStore.has("nyxid.push.pending_unbind")).toBe(false);
});

test("pending rebind rotates a changed native token from the stored token", async () => {
  setPushDeliverySuppressed(true);
  mockStore.set(
    "nyxid.push.pending_unbind",
    JSON.stringify({
      token: "pending-token",
      provider: "apns",
      platform: "ios",
      createdAt: new Date().toISOString(),
    }),
  );
  jest.mocked(Notifications.getDevicePushTokenAsync).mockResolvedValueOnce({
    type: "ios",
    data: "new-native-token",
  } as never);
  await activatePushAfterLogin();
  expect(mobileApi.registerPushToken).toHaveBeenCalledTimes(1);
  expect(mobileApi.rotatePushToken).toHaveBeenCalledWith(
    expect.objectContaining({
      token: "new-native-token",
      previous_token: "pending-token",
    }),
  );
  expect(mockStore.get("nyxid.push.device_token")).toBe("new-native-token");
});

test("failed pending rebind never fetches a native token and active retries the record", async () => {
  setPushDeliverySuppressed(true);
  mockStore.set(
    "nyxid.push.pending_unbind",
    JSON.stringify({
      token: "pending-token",
      provider: "apns",
      platform: "ios",
      createdAt: new Date().toISOString(),
    }),
  );
  const warning = jest.spyOn(console, "warn").mockImplementation(() => {});
  jest
    .mocked(mobileApi.registerPushToken)
    .mockRejectedValueOnce(new Error("offline"))
    .mockResolvedValueOnce({ registered: true } as never);
  await activatePushAfterLogin();
  expect(Notifications.getDevicePushTokenAsync).not.toHaveBeenCalled();
  expect(mockStore.has("nyxid.push.pending_unbind")).toBe(true);
  const stop = await initializeNotificationRuntime();
  expect(appStateListener).toBeDefined();
  appStateListener!("active");
  await new Promise((resolve) => setTimeout(resolve, 10));
  expect(mobileApi.registerPushToken).toHaveBeenCalledTimes(2);
  expect(mockStore.has("nyxid.push.pending_unbind")).toBe(false);
  stop();
  warning.mockRestore();
});
test("runtime token refresh does no API work while signed out; next login registers it", async () => {
  const stop = await initializeNotificationRuntime();
  setAuthAccountId(null);
  mockRefresh({ type: "ios", data: "same-native-token" });
  await Promise.resolve();
  expect(mobileApi.registerPushToken).not.toHaveBeenCalled();
  expect(mobileApi.rotatePushToken).not.toHaveBeenCalled();
  setAuthAccountId("two");
  await activatePushAfterLogin();
  expect(mobileApi.registerPushToken).toHaveBeenCalledTimes(1);
  stop();
});

function seedPending(createdAt = new Date().toISOString()) {
  setPushDeliverySuppressed(true);
  mockStore.set(
    "nyxid.push.pending_unbind",
    JSON.stringify({
      token: "pending-token", provider: "apns", platform: "ios", createdAt,
    }),
  );
}
function pushError(statusCode: number) {
  return new ApiError({
    statusCode, errorCode: 1000, errorKey: "rejected", message: "Rejected",
  });
}
const visibleNotification = (id: string) => ({
  request: {
    identifier: id, content: { title: "Approval", body: "Review", data: {} },
  },
});

test("notification handling cannot clear suppression while a pending record write is delayed", async () => {
  const stop = await initializeNotificationRuntime();
  AppState.currentState = "inactive";
  mockStore.set("nyxid.push.device_token", "same-native-token");
  jest
    .mocked(mobileApi.unregisterPushToken)
    .mockRejectedValueOnce(new Error("offline"));
  let release!: () => void;
  let writing!: () => void;
  const entered = new Promise<void>((resolve) => {
    writing = resolve;
  });
  const blocked = new Promise<void>((resolve) => {
    release = resolve;
  });
  jest
    .mocked(SecureStore.setItemAsync)
    .mockImplementationOnce(async (key, value) => {
      writing();
      await blocked;
      mockStore.set(key, value);
    });
  const logout = deactivatePushOnLogout();
  await entered;
  expect(isPushDeliverySuppressed()).toBe(true);
  const presentation = await mockHandler!.handleNotification(
    visibleNotification("setting") as never,
  );
  expect(presentation.shouldShowAlert).toBe(false);
  expect(isPushDeliverySuppressed()).toBe(true);
  release();
  await logout;
  stop();
});

test("notification handling cannot restore suppression while a pending record clear is delayed", async () => {
  seedPending();
  const stop = await initializeNotificationRuntime();
  AppState.currentState = "inactive";
  let release!: () => void;
  let clearing!: () => void;
  const entered = new Promise<void>((resolve) => {
    clearing = resolve;
  });
  const blocked = new Promise<void>((resolve) => {
    release = resolve;
  });
  jest.mocked(SecureStore.deleteItemAsync).mockImplementationOnce(async (key) => {
    clearing();
    await blocked;
    mockStore.delete(key);
  });
  const registration = activatePushAfterLogin();
  await entered;
  expect(isPushDeliverySuppressed()).toBe(false);
  const presentation = await mockHandler!.handleNotification(
    visibleNotification("clearing") as never,
  );
  expect(presentation.shouldShowAlert).toBe(true);
  expect(isPushDeliverySuppressed()).toBe(false);
  release();
  await registration;
  stop();
});

test.each([400, 403, 404, 409, 422])(
  "terminal rebind HTTP %s drops suppression and attempts normal registration",
  async (status) => {
    seedPending();
    jest
      .mocked(mobileApi.registerPushToken)
      .mockRejectedValueOnce(pushError(status))
      .mockResolvedValueOnce({ registered: true } as never);
    const result = await activatePushAfterLogin();
    expect(result.registered).toBe(true);
    expect(mockStore.has("nyxid.push.pending_unbind")).toBe(false);
    expect(isPushDeliverySuppressed()).toBe(false);
    expect(mobileApi.registerPushToken).toHaveBeenCalledTimes(2);
    expect(mobileApi.registerPushToken).toHaveBeenLastCalledWith(
      expect.objectContaining({ token: "same-native-token" }),
    );
  },
);

test.each([401, 408, 429, 503])(
  "retryable rebind HTTP %s keeps the pending record and suppression",
  async (status) => {
    seedPending();
    const warning = jest.spyOn(console, "warn").mockImplementation(() => {});
    jest.mocked(mobileApi.registerPushToken).mockRejectedValueOnce(pushError(status));
    expect((await activatePushAfterLogin()).registered).toBe(false);
    expect(mockStore.has("nyxid.push.pending_unbind")).toBe(true);
    expect(isPushDeliverySuppressed()).toBe(true);
    expect(Notifications.getDevicePushTokenAsync).not.toHaveBeenCalled();
    expect(warning).toHaveBeenCalledWith(
      "[push] pending token rebind failed", expect.any(ApiError),
    );
  },
);

test("a terminal rebind failure surfaces the normal registration result", async () => {
  seedPending();
  const warning = jest.spyOn(console, "warn").mockImplementation(() => {});
  const capError = pushError(400);
  jest.mocked(mobileApi.registerPushToken)
    .mockRejectedValueOnce(capError)
    .mockRejectedValueOnce(capError);
  expect(await activatePushAfterLogin()).toMatchObject({
    registered: false, reason: "register_failed",
  });
  expect(mobileApi.registerPushToken).toHaveBeenCalledTimes(2);
  expect(mockStore.has("nyxid.push.pending_unbind")).toBe(false);
  expect(isPushDeliverySuppressed()).toBe(false);
  expect(warning).toHaveBeenCalledWith("[push] register token failed", capError);
});

test("a pending record older than seven days gives up and uses normal registration", async () => {
  seedPending(new Date(Date.now() - 7 * 86400000 - 1).toISOString());
  jest
    .mocked(mobileApi.registerPushToken)
    .mockResolvedValueOnce({ registered: true } as never);
  expect((await activatePushAfterLogin()).registered).toBe(true);
  expect(mockStore.has("nyxid.push.pending_unbind")).toBe(false);
  expect(isPushDeliverySuppressed()).toBe(false);
  expect(mobileApi.registerPushToken).toHaveBeenCalledTimes(1);
  expect(mobileApi.registerPushToken).toHaveBeenCalledWith(
    expect.objectContaining({ token: "same-native-token" }),
  );
});

test("native unregister retries twice before its third attempt succeeds", async () => {
  mockStore.set("nyxid.push.device_token", "same-native-token");
  jest
    .mocked(mobileApi.unregisterPushToken)
    .mockRejectedValueOnce(new Error("offline"));
  jest
    .mocked(Notifications.unregisterForNotificationsAsync)
    .mockRejectedValueOnce(new Error("busy"))
    .mockRejectedValueOnce(new Error("busy"));
  expect(await deactivatePushOnLogout()).toBe(false);
  expect(Notifications.unregisterForNotificationsAsync).toHaveBeenCalledTimes(3);
  expect(
    JSON.parse(mockStore.get("nyxid.push.pending_unbind")!).nativeUnregisterPending,
  ).toBe(false);
  expect(isPushDeliverySuppressed()).toBe(true);
});

test("exhausted native unregister is retained and active retries even while signed out", async () => {
  const stop = await initializeNotificationRuntime();
  const warning = jest.spyOn(console, "warn").mockImplementation(() => {});
  mockStore.set("nyxid.push.device_token", "same-native-token");
  jest
    .mocked(mobileApi.unregisterPushToken)
    .mockRejectedValueOnce(new Error("offline"));
  jest
    .mocked(Notifications.unregisterForNotificationsAsync)
    .mockRejectedValue(new Error("busy"));
  await deactivatePushOnLogout();
  expect(Notifications.unregisterForNotificationsAsync).toHaveBeenCalledTimes(3);
  expect(
    JSON.parse(mockStore.get("nyxid.push.pending_unbind")!).nativeUnregisterPending,
  ).toBe(true);
  setAuthAccountId(null);
  appStateListener!("active");
  await new Promise((resolve) => setTimeout(resolve, 400));
  expect(Notifications.unregisterForNotificationsAsync).toHaveBeenCalledTimes(6);
  expect(mockStore.has("nyxid.push.pending_unbind")).toBe(true);
  expect(isPushDeliverySuppressed()).toBe(true);
  expect(mobileApi.registerPushToken).not.toHaveBeenCalled();
  expect(warning).toHaveBeenCalledTimes(2);
  stop();
});


test('M2622-03 post-tombstone teardown uses captured credentials and persists pending unbind before network', async () => {
  mockStore.set('nyxid.push.device_token', 'native-token');
  const captured = await capturePushLogout('old-access');
  setAuthAccountId(null);
  let release!: () => void;
  jest.mocked(mobileApi.unregisterPushToken).mockImplementationOnce(async (_payload, accessToken) => {
    expect(accessToken).toBe('old-access');
    expect(mockStore.has('nyxid.push.pending_unbind')).toBe(true);
    await new Promise<void>(resolve => {release=resolve;});
  });
  const operation=deactivatePushOnLogout(captured);
  for(let i=0;i<50 && !release;i++) await Promise.resolve();
  expect(release).toBeDefined();
  expect(isPushDeliverySuppressed()).toBe(true);
  release(); await operation;
  expect(mockStore.has('nyxid.push.pending_unbind')).toBe(false);
});

test('M2622-03 missing capture and unreadable push storage still invalidate native delivery after durable clear', async () => {
  const read = jest.mocked(SecureStore.getItemAsync).getMockImplementation()!;
  setAuthAccountId(null);
  jest.mocked(SecureStore.getItemAsync).mockImplementation(async (key) => {
    if (key === 'nyxid.push.device_token') throw Error('push storage unavailable');
    return read(key);
  });
  try {
    await expect(deactivatePushOnLogout({accessToken: 'rotated-access', registration: null})).resolves.toBe(false);
    expect(Notifications.unregisterForNotificationsAsync).toHaveBeenCalled();
  } finally {jest.mocked(SecureStore.getItemAsync).mockImplementation(read);}
});
