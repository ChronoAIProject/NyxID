import assert from "node:assert/strict";
import test from "node:test";
import { URL, fileURLToPath } from "node:url";
import * as core from "@react-navigation/core";
import {
  hookHarness,
  deferred,
} from "../features/assistant/assistantHookTestHarness";
import * as destination from "../lib/notifications/notificationDestination";
import * as hierarchy from "./navigationHierarchy";
import * as canonical from "./workspaceCanonicalLinks";
import * as config from "./navigationLinking";
import { fixture } from "./workspaceNavigationTestHarness";

type Hook = ReturnType<
  typeof import("./useNotificationNavigation").useNotificationNavigation
>;
function response(
  data: unknown,
  identifier = "one",
  trigger: unknown = null,
): any {
  return {
    actionIdentifier: "default",
    notification: { request: { identifier, content: { data }, trigger } },
  };
}
function setup() {
  let ready = false;
  let state: any = { ...fixture(), routes: [{ name: "Auth" }] };
  const guards = { current: new Set<symbol>() };
  const actions: any[] = [];
  const cached = deferred<any>();
  let tap!: (response: any) => void;
  let unsubscribed = false;
  let cleared = 0;
  const h = hookHarness<Hook>({
    "@react-navigation/native": core,
    "expo-notifications": {
      getLastNotificationResponseAsync: () => cached.promise,
      clearLastNotificationResponseAsync: async () => {
        cleared++;
      },
      addNotificationResponseReceivedListener: (cb: typeof tap) => {
        tap = cb;
        return {
          remove: () => {
            unsubscribed = true;
          },
        };
      },
    },
    "react-native": { Linking: { getInitialURL: async () => null } },
    "../lib/env": {
      NYXID_APP_SCHEME: "nyxid",
      NYXID_TRUSTED_FRONTEND_URLS: ["https://nyx.example"],
      NYXID_TRUSTED_API_BASE_URLS: [],
      IS_DEV_BUILD: false,
    },
    "../lib/linkingPrefixes": {
      buildLinkingPrefixes: () => ["nyxid://", "https://nyx.example"],
    },
    "../lib/telemetry": { capture: async () => {} },
    "../lib/notifications/notificationDestination": destination,
    "./navigationHierarchy": hierarchy,
    "./navigationLinking": config,
    "./workspaceCanonicalLinks": canonical,
  });
  const linking = h.load(
    fileURLToPath(new URL("./linking.ts", import.meta.url)),
  ) as any;
  h.mock("./linking", linking);
  const hook = h.load(
    fileURLToPath(new URL("./useNotificationNavigation.ts", import.meta.url)),
  ).useNotificationNavigation as any;
  h.mount(() =>
    hook(
      {
        isReady: () => ready,
        getRootState: () => state,
        dispatch: (action: any) => actions.push(action),
      },
      guards,
    ),
  );
  return {
    h,
    guards,
    actions,
    cached,
    linking,
    tap: (r: any) => tap(r),
    cleared: () => cleared,
    unsubscribed: () => unsubscribed,
    signIn: () => {
      ready = true;
      state = fixture();
      h.value().flush();
    },
  };
}
test("cold notification waits through sign-in and navigation readiness and is delivered once", async () => {
  const f = setup();
  f.cached.resolve(response({ conversation_id: "thread" }));
  await f.h.settle();
  assert.equal(f.actions.length, 0);
  f.signIn();
  assert.equal(f.actions.length, 1);
  assert.match(JSON.stringify(f.actions[0]), /thread/);
  f.tap(response({ conversation_id: "thread" }));
  assert.equal(f.actions.length, 1);
  assert.equal(f.cleared(), 1);
  f.h.unmount();
  assert.equal(f.unsubscribed(), true);
});
test("warm tap and foreground Open share guards; newest destination replaces an older approval", async () => {
  const f = setup();
  f.signIn();
  const guard = Symbol();
  f.guards.current.add(guard);
  f.tap(response({ type: "approval_request", request_id: "approval" }));
  f.h.value().open("https://nyx.example/assistant?c=nyxg-crew");
  assert.equal(f.actions.length, 0);
  f.guards.current.delete(guard);
  f.h.value().flush();
  assert.equal(f.actions.length, 1);
  assert.match(JSON.stringify(f.actions[0]), /GroupChat/);
  assert.doesNotMatch(JSON.stringify(f.actions[0]), /approval/);
  // A late asynchronous cold read cannot replace a newer warm tap.
  f.cached.resolve(response({ conversation_id: "stale" }, "stale"));
  await f.h.settle();
  assert.equal(f.actions.length, 1);
  f.tap(response({ type: "approval_request", request_id: "review" }, "review"));
  assert.equal(f.actions.length, 2);
  assert.match(JSON.stringify(f.actions[1]), /review/);
  f.h.unmount();
});
test("actual response extraction covers APNs trigger payload and FCM remote message data", () => {
  const f = setup();
  for (const trigger of [
    { type: "push", payload: { data: { group_id: "crew" } } },
    { type: "push", remoteMessage: { data: { group_id: "crew" } } },
  ]) {
    assert.equal(
      f.linking.extractUrlFromNotificationResponse(
        response({}, "push", trigger),
      ),
      "nyxid://groups/crew",
    );
  }
  f.h.unmount();
});

test("a foreground Open fences a late cached cold response while navigation is guarded", async () => {
  const f = setup();
  f.signIn();
  const guard = Symbol();
  f.guards.current.add(guard);
  f.h.value().open("https://nyx.example/assistant?c=foreground-thread");
  f.cached.resolve(response({ conversation_id: "stale-cold-thread" }));
  await f.h.settle();
  assert.equal(f.actions.length, 0);
  f.guards.current.delete(guard);
  f.h.value().flush();
  assert.equal(f.actions.length, 1);
  assert.match(JSON.stringify(f.actions[0]), /foreground-thread/);
  assert.doesNotMatch(JSON.stringify(f.actions[0]), /stale-cold-thread/);
  f.h.unmount();
});
