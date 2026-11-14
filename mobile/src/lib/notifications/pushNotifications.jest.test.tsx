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
  addNotificationReceivedListener: () => ({ remove() {} }),
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
