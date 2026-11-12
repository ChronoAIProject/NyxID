import assert from "node:assert/strict";
import test from "node:test";
import { fileURLToPath, URL } from "node:url";
import { hookHarness } from "../../features/assistant/assistantHookTestHarness";
import * as destination from "./notificationDestination";

test("actual foreground receive listener survives malformed-relative URLs and supplies type/Activity Open fallbacks", async () => {
  let receive!: (notification: any) => void;
  const h = hookHarness(
    {
      "./notificationDestination": destination,
      "../env": {
        NYXID_APP_SCHEME: "nyxid",
        NYXID_TRUSTED_FRONTEND_URLS: ["https://nyx.example"],
        NYXID_TRUSTED_API_BASE_URLS: [],
      },
      "expo-notifications": {
        setNotificationHandler: () => {},
        registerTaskAsync: async () => {},
        addNotificationReceivedListener: (cb: typeof receive) => {
          receive = cb;
          return { remove: () => {} };
        },
      },
      "expo-task-manager": {
        isTaskDefined: () => true,
        isTaskRegisteredAsync: async () => true,
      },
      "expo-secure-store": {},
      "react-native": {
        AppState: { currentState: "active" },
        Platform: { OS: "ios" },
      },
      "../api/mobileApi": {},
      "./approvalRefreshSignal": {
        signalApprovalStateMayHaveChanged: () => {},
      },
    },
    { __DEV__: false },
  );
  const module = h.load(
    fileURLToPath(new URL("./pushNotifications.ts", import.meta.url)),
  ) as typeof import("./pushNotifications");
  const displayed: any[] = [];
  const unsubscribe = module.setForegroundNotificationHandler((n) =>
    displayed.push(n),
  );
  const cleanup = await module.initializeNotificationRuntime();
  for (const type of ["channel_bot_failed", "unknown"]) {
    receive({
      request: {
        content: { title: "Review update", data: { type, url: "//[" } },
      },
    });
  }
  assert.deepEqual(
    displayed.map((n) => n.url),
    ["nyxid://plugins?tab=connectors", "nyxid://activity"],
  );
  unsubscribe();
  cleanup();
});
