import * as Notifications from "expo-notifications";
import * as SecureStore from "expo-secure-store";
import { mobileApi } from "../api/mobileApi";
import { setAuthAccountId } from "../auth/authAccount";
import {
  activatePushAfterLogin,
  deactivatePushOnLogout,
  initializeNotificationRuntime,
} from "./pushNotifications";
const mockStore = new Map<string, string>();
let mockRefresh: (token: unknown) => void;
let notificationListener: ((notification: any) => void) | undefined;
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
jest.mock("expo-notifications", () => ({
  setNotificationHandler: jest.fn(),
  addNotificationReceivedListener: (listener: (notification: any) => void) => {
    notificationListener = listener;
    return { remove() {} };
  },
  addPushTokenListener: (listener: typeof mockRefresh) => {
    mockRefresh = listener;
    return { remove() {} };
  },
  unregisterForNotificationsAsync: jest.fn(async () => {}),
  dismissAllNotificationsAsync: jest.fn(async () => {}),
  setBadgeCountAsync: jest.fn(async () => true),
  clearLastNotificationResponseAsync: jest.fn(async () => {}),
  getPermissionsAsync: async () => ({ granted: true, status: "granted" }),
  getDevicePushTokenAsync: async () => ({
    type: "ios",
    data: "same-native-token",
  }),
}));
beforeEach(() => {
  mockStore.clear();
  setAuthAccountId("one");
  jest.clearAllMocks();
  jest
    .mocked(mobileApi.unregisterPushToken)
    .mockResolvedValue({ deleted: true } as never);
});
afterEach(() => setAuthAccountId(null));

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
    .mockRejectedValueOnce(new Error("unsupported runtime"));
  await expect(deactivatePushOnLogout()).resolves.toBe(false);
  expect(warning).toHaveBeenCalledWith("[push] native unregister unavailable");
  warning.mockRestore();
  expect(mockStore.has("nyxid.push.device_token")).toBe(false);
});

test("logout waits for an in-flight registration before sending the authenticated unbind", async () => {
  let settleRegistration!: () => void;
  jest.mocked(mobileApi.registerPushToken).mockImplementationOnce(
    () => new Promise((resolve) => {
      settleRegistration = () => resolve({ registered: true } as never);
    }),
  );
  mockStore.set("nyxid.push.device_token", "same-native-token");
  const registration = activatePushAfterLogin({ forceRegister: true });
  for (let index = 0; index < 8; index += 1) await Promise.resolve();
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
  const handler = jest.mocked(Notifications.setNotificationHandler).mock.calls.at(-1)?.[0];
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
  stop();
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
