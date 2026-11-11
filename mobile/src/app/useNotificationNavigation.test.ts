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
import * as authEpoch from "../lib/auth/authEpoch";
import * as authAccount from "../lib/auth/authAccount";

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
function setup(initialAccount: string | null = null) {
  let ready = false;
  let accountId = initialAccount;
  authAccount.setAuthAccountId(initialAccount);
  const credentials = new Map<string, string>();
  const token = (sub: string, revision = 0) =>
    `e30.${Buffer.from(JSON.stringify({ sub, revision })).toString("base64url")}.sig`;
  const telemetry: any[] = [];
  let state: any = { ...fixture(), routes: [{ name: "Auth" }] };
  const guards = { current: new Set<symbol>() };
  const actions: any[] = [];
  const cached = deferred<any>();
  let initialUrl: string | null = null;
  let directLink: ((event: { url: string }) => void) | undefined;
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
    "react-native": {
      Linking: {
        getInitialURL: async () => initialUrl,
        addEventListener: (_: string, listener: typeof directLink) => {
          directLink = listener;
          return {
            remove: () => {
              directLink = undefined;
            },
          };
        },
      },
    },
    "../lib/env": {
      NYXID_APP_SCHEME: "nyxid",
      NYXID_TRUSTED_FRONTEND_URLS: ["https://nyx.example"],
      NYXID_TRUSTED_API_BASE_URLS: [],
      IS_DEV_BUILD: false,
    },
    "../lib/linkingPrefixes": {
      buildLinkingPrefixes: () => ["nyxid://", "https://nyx.example"],
    },
    "../lib/telemetry": {
      capture: async (event: any) => {
        telemetry.push(event);
      },
    },
    "./authEpoch": authEpoch,
    "./authAccount": authAccount,
    "expo-secure-store": {
      getItemAsync: async (key: string) => credentials.get(key) ?? null,
      setItemAsync: async (key: string, value: string) => {
        credentials.set(key, value);
      },
      deleteItemAsync: async (key: string) => {
        credentials.delete(key);
      },
    },
    "../analyticsConsentState": {
      analyticsConsentState: { bindAccount: () => {} },
    },
    "../lib/auth/authAccount": authAccount,
    "../lib/notifications/notificationDestination": destination,
    "./navigationHierarchy": hierarchy,
    "./navigationLinking": config,
    "./workspaceCanonicalLinks": canonical,
  });
  const sessions = h.load(
    fileURLToPath(new URL("../lib/auth/sessionStore.ts", import.meta.url)),
  ) as typeof import("../lib/auth/sessionStore");
  const linking = h.load(
    fileURLToPath(new URL("./linking.ts", import.meta.url)),
  ) as any;
  h.mock("./linking", linking);
  const hook = h.load(
    fileURLToPath(new URL("./useNotificationNavigation.ts", import.meta.url)),
  ).useNotificationNavigation as any;
  const navigation = {
    isReady: () => ready,
    getRootState: () => state,
    dispatch: (action: any) => actions.push(action),
  };
  h.mount(() => hook(navigation, guards));
  return {
    h,
    guards,
    actions,
    telemetry,
    cached,
    linking,
    directLink: (url: string) => {
      assert.ok(directLink, "the real appLinking subscription must be mounted");
      directLink({ url });
    },
    setInitialUrl: (url: string) => {
      initialUrl = url;
    },
    tap: (r: any) => tap(r),
    cleared: () => cleared,
    unsubscribed: () => unsubscribed,
    clearCredentials: sessions.clearStoredAuthSession,
    signIn: async (nextAccount = "account-a") => {
      // AuthSessionContext advances before persistence; sessionStore advances
      // again. Exercise that real module instead of a separate test counter.
      authEpoch.advanceAuthEpoch();
      if (authAccount.getAuthAccountId() !== nextAccount)
        authAccount.setAuthAccountId(null);
      await sessions.persistAuthSession({
        accessToken: token(nextAccount),
        refreshToken: "refresh",
      });
      accountId = nextAccount;
      authAccount.setAuthAccountId(nextAccount);
      ready = true;
      state = fixture();
      h.render();
      h.value().flush();
    },
    signOut: async () => {
      authEpoch.advanceAuthEpoch();
      accountId = null;
      authAccount.setAuthAccountId(null);
      h.render();
      await sessions.clearStoredAuthSession();
      state = { ...fixture(), routes: [{ name: "Auth" }] };
      h.value().flush();
    },
    refresh: async () => {
      const session = await sessions.loadStoredAuthSession();
      assert.ok(session && accountId);
      assert.equal(
        await sessions.commitRefreshedAuthSession(session, {
          ...session,
          accessToken: token(accountId, 1),
        }),
        true,
      );
      h.render();
    },
  };
}
test("cold notification waits through sign-in and navigation readiness and is delivered once", async () => {
  const f = setup();
  f.cached.resolve(response({ conversation_id: "thread" }));
  await f.h.settle();
  assert.equal(f.actions.length, 0);
  const beforeSignIn = authEpoch.getAuthEpoch();
  await f.signIn();
  assert.equal(authEpoch.getAuthEpoch(), beforeSignIn + 2);
  assert.equal(f.actions.length, 1);
  assert.match(JSON.stringify(f.actions[0]), /thread/);
  f.tap(response({ conversation_id: "thread" }));
  assert.equal(f.actions.length, 1);
  assert.equal(f.cleared(), 1);
  assert.deepEqual(JSON.parse(JSON.stringify(f.telemetry)), [
    { name: "mobile.deep_link_opened", props: { link_type: "other" } },
  ]);
  f.h.unmount();
  assert.equal(f.unsubscribed(), true);
});
test("warm tap and foreground Open share guards; newest destination replaces an older approval", async () => {
  const f = setup();
  await f.signIn();
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

for (const url of [
  "nyxid://assistant?c=newer-direct-thread",
  "https://nyx.example/assistant?c=newer-direct-thread",
]) {
  test(`a direct link supersedes a guarded notification: ${url}`, async () => {
    const f = setup();
    await f.signIn();
    const guard = Symbol();
    f.guards.current.add(guard);
    f.tap(response({ conversation_id: "older-notification-thread" }));
    const directUrls: string[] = [];
    const unsubscribe = f.linking.appLinking.subscribe((next: string) =>
      directUrls.push(next),
    );
    f.directLink(url);
    assert.deepEqual(directUrls, [url]);
    f.guards.current.delete(guard);
    f.h.value().flush();
    assert.equal(
      f.actions.length,
      0,
      "the old notification must not override the direct route",
    );
    f.tap(response({ conversation_id: "newest-notification-thread" }, "newest"));
    assert.equal(f.actions.length, 1);
    assert.match(JSON.stringify(f.actions[0]), /newest-notification-thread/);
    unsubscribe();
    f.h.unmount();
  });
}

test("a direct link fences a late cached notification response", async () => {
  const f = setup();
  await f.signIn();
  const unsubscribe = f.linking.appLinking.subscribe(() => {});
  f.directLink("nyxid://challenge/newer-direct-approval");
  f.cached.resolve(response({ conversation_id: "stale-cold-thread" }));
  await f.h.settle();
  f.h.value().flush();
  assert.equal(f.actions.length, 0);
  unsubscribe();
  f.h.unmount();
});

test("an initial direct link supersedes a notification waiting for navigation", async () => {
  const f = setup();
  f.cached.resolve(response({ conversation_id: "older-cold-thread" }));
  await f.h.settle();
  f.setInitialUrl("nyxid://challenge/initial-direct-approval");
  assert.equal(
    await f.linking.appLinking.getInitialURL(),
    "nyxid://activity?challengeId=initial-direct-approval",
  );
  await f.signIn();
  assert.equal(f.actions.length, 0);
  f.h.unmount();
});

test("an untrusted direct URL does not discard a guarded notification", async () => {
  const f = setup();
  await f.signIn();
  const guard = Symbol();
  f.guards.current.add(guard);
  f.tap(response({ conversation_id: "trusted-notification-thread" }));
  const unsubscribe = f.linking.appLinking.subscribe(() => {});
  f.directLink("https://untrusted.example/assistant?c=ignored");
  f.guards.current.delete(guard);
  f.h.value().flush();
  assert.equal(f.actions.length, 1);
  assert.match(JSON.stringify(f.actions[0]), /trusted-notification-thread/);
  unsubscribe();
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
  await f.signIn();
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

test("warm and toast taps report only link_type once and stale cold reads do not report", async () => {
  const f = setup();
  await f.signIn();
  f.tap(response({ type: "approval_request", request_id: "secret-id" }));
  f.tap(response({ type: "approval_request", request_id: "secret-id" }));
  f.h.value().open("https://nyx.example/assistant?c=secret-thread");
  f.cached.resolve(response({ conversation_id: "stale" }, "cold"));
  await f.h.settle();
  assert.deepEqual(JSON.parse(JSON.stringify(f.telemetry)), [
    { name: "mobile.deep_link_opened", props: { link_type: "challenge" } },
    { name: "mobile.deep_link_opened", props: { link_type: "other" } },
  ]);
  f.h.unmount();
});
for (const delayed of [false, true]) {
  test(`${delayed ? "delayed cold read" : "pending tap"} accepted signed out survives the next sign-in's real epoch advances`, async () => {
    const f = setup();
    if (!delayed) {
      f.cached.resolve(response({ conversation_id: "signed-out-thread" }));
      await f.h.settle();
    }
    const before = authEpoch.getAuthEpoch();
    await f.signIn();
    assert.equal(authEpoch.getAuthEpoch(), before + 2);
    if (delayed) {
      f.cached.resolve(response({ conversation_id: "signed-out-thread" }));
      await f.h.settle();
    }
    assert.equal(f.actions.length, 1);
    assert.match(JSON.stringify(f.actions[0]), /signed-out-thread/);
    f.h.unmount();
  });
  for (const transition of [
    "sign-out",
    "account-switch",
    "sign-out-and-back",
  ]) {
    test(`${delayed ? "delayed cold read" : "pending tap"} bound to A is dropped on ${transition}`, async () => {
      const f = setup("account-a");
      await f.signIn();
      const guard = Symbol();
      f.guards.current.add(guard);
      if (!delayed) f.tap(response({ conversation_id: "private-a-thread" }));
      if (transition !== "account-switch") await f.signOut();
      if (transition !== "sign-out")
        await f.signIn(
          transition === "account-switch" ? "account-b" : "account-a",
        );
      if (delayed) {
        f.cached.resolve(response({ conversation_id: "private-a-thread" }));
        await f.h.settle();
        assert.equal(f.telemetry.length, 0);
        assert.equal(
          f.cleared(),
          1,
          "a discarded cold response cannot replay after a restart",
        );
      }
      f.guards.current.delete(guard);
      f.h.value().flush();
      assert.equal(f.actions.length, 0);
      f.h.unmount();
    });
  }
  test(`${delayed ? "delayed cold read" : "pending tap"} survives a real same-account token refresh`, async () => {
    const f = setup("account-a");
    await f.signIn();
    const guard = Symbol();
    f.guards.current.add(guard);
    if (!delayed) f.tap(response({ conversation_id: "refresh-thread" }));
    const before = authEpoch.getAuthEpoch();
    await f.refresh();
    assert.equal(authEpoch.getAuthEpoch(), before);
    if (delayed) {
      f.cached.resolve(response({ conversation_id: "refresh-thread" }));
      await f.h.settle();
    }
    f.guards.current.delete(guard);
    f.h.value().flush();
    assert.equal(f.actions.length, 1);
    assert.match(JSON.stringify(f.actions[0]), /refresh-thread/);
    f.h.unmount();
  });
}
test("an anonymous delayed cold read binds to the first sign-in, not a later account", async () => {
  const f = setup();
  await f.signIn();
  await f.signIn("account-b");
  f.cached.resolve(response({ conversation_id: "first-account-thread" }));
  await f.h.settle();
  assert.equal(f.actions.length, 0);
  assert.equal(f.telemetry.length, 0);
  assert.equal(f.cleared(), 1);
  f.h.unmount();
});
test("sign-out and same-account admission invalidate a tap even without a React render between them", async () => {
  const f = setup("account-a");
  await f.signIn();
  const guard = Symbol();
  f.guards.current.add(guard);
  f.tap(response({ conversation_id: "old-session-thread" }));
  authEpoch.advanceAuthEpoch();
  authAccount.setAuthAccountId(null);
  authEpoch.advanceAuthEpoch();
  authAccount.setAuthAccountId("account-a");
  f.guards.current.delete(guard);
  f.h.value().flush();
  assert.equal(f.actions.length, 0);
  f.h.unmount();
});
test("HTTP credential invalidation drops account-bound taps without waiting for auth UI", async () => {
  const f = setup("account-a");
  await f.signIn();
  const guard = Symbol();
  f.guards.current.add(guard);
  f.tap(response({ conversation_id: "invalidated-thread" }));
  const before = authEpoch.getAuthEpoch();
  await f.clearCredentials();
  assert.equal(authEpoch.getAuthEpoch(), before + 1);
  assert.equal(authAccount.getAuthAccountId(), null);
  await f.signIn();
  f.guards.current.delete(guard);
  f.h.value().flush();
  assert.equal(f.actions.length, 0);
  f.h.unmount();
});
test("unrecognized destinations and empty payload taps fall back to native Activity", async () => {
  const f = setup();
  await f.signIn();
  f.h.value().open("https://nyx.example/no-native-route");
  f.tap(response({}));
  assert.equal(f.actions.length, 2);
  assert.ok(
    f.actions.every((action) => /Activity/.test(JSON.stringify(action))),
  );
  f.h.value().flush();
  assert.equal(f.actions.length, 2);
  f.h.unmount();
});

for (const cold of [false, true]) {
  for (const known of [false, true]) {
    test(`${cold ? "cold" : "warm"} malformed-relative notification uses ${known ? "known type" : "Activity"} fallback`, async () => {
      const f = setup();
      const tap = response({ type: known ? "channel_bot_failed" : "unknown", url: "//[" });
      if (cold) f.cached.resolve(tap);
      else { f.cached.resolve(null); f.tap(tap); }
      await f.h.settle();
      await f.signIn();
      assert.equal(f.actions.length, 1);
      assert.match(JSON.stringify(f.actions[0]), known ? /Plugins/ : /Activity/);
      f.h.unmount();
    });
  }
}
