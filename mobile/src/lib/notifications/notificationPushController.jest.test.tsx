import { QueryClient } from "@tanstack/react-query";
import { createNotificationsApi } from "../api/notificationsTransport";
import { NOTIFICATIONS } from "../../features/notifications/notificationKeys";
import {
  createNotificationPushController,
  notificationDeeplinkPath,
} from "./notificationPushController";

const ORIGINS = ["https://app.nyxid.test", "https://app.nyxid.dev"];
const C = `nyxa-${"a".repeat(32)}`;
const AGENT = "7c0f6a8e-2b2e-4a1a-9f1f-0d2c5a6b7c8d";
const flushMicrotasks = () => new Promise((resolve) => setTimeout(resolve, 0));
// Every cached query arms a garbage-collection timer; clearing the clients
// after each test lets Jest exit as soon as the run ends.
const clients: QueryClient[] = [];
afterEach(() => {
  for (const client of clients.splice(0)) client.clear();
});

function harness(options: { ready?: boolean; foreground?: boolean; online?: boolean } = {}) {
  const requests: { path: string; method?: string; body?: unknown }[] = [];
  let failRead = false;
  const api = createNotificationsApi(async (path, init) => {
    requests.push({ path, method: init?.method, body: init?.body });
    if (failRead) throw new Error("offline");
    return {};
  });
  const queryClient = new QueryClient();
  clients.push(queryClient);
  const summaryKey = [...NOTIFICATIONS, "summary", 1];
  const feedKey = [...NOTIFICATIONS, "feed", 1, "all"];
  const otherKey = ["challenges"];
  queryClient.setQueryData(summaryKey, { unread_count: 1 });
  queryClient.setQueryData(feedKey, { pages: [] });
  queryClient.setQueryData(otherKey, { total: 0 });
  const opened: string[] = [];
  const state = { ready: options.ready ?? true, foreground: options.foreground ?? true, online: options.online ?? true };
  const controller = createNotificationPushController({
    api,
    queryClient,
    frontendUrls: ORIGINS,
    appScheme: "nyxid",
    isForeground: () => state.foreground,
    isOnline: () => state.online,
    openPath: (path) => {
      if (!state.ready) return false;
      // The navigator records the dispatch; a read request must not precede it.
      opened.push(`${path}@${requests.length}`);
      return true;
    },
  });
  const invalidated = (key: unknown[]) => queryClient.getQueryState(key)?.isInvalidated === true;
  return {
    controller,
    requests,
    opened,
    state,
    invalidated,
    summaryKey,
    feedKey,
    otherKey,
    setFailRead: (value: boolean) => {
      failRead = value;
    },
  };
}

describe("notificationDeeplinkPath", () => {
  it("accepts first-party https links and the app scheme, nothing else", () => {
    expect(notificationDeeplinkPath(`https://app.nyxid.test/assistant?c=${C}`, ORIGINS, "nyxid")).toBe(`/assistant?c=${C}`);
    expect(notificationDeeplinkPath(`https://app.nyxid.dev/assistant?agent=${AGENT}&details=channels`, ORIGINS, "nyxid")).toBe(
      `/assistant?agent=${AGENT}&details=channels`,
    );
    expect(notificationDeeplinkPath(`nyxid://assistant?c=${C}`, ORIGINS, "nyxid")).toBe(`/assistant?c=${C}`);
    expect(notificationDeeplinkPath("nyxid://assistant/automations?agent=a", ORIGINS, "nyxid")).toBe("/assistant/automations?agent=a");
    for (const link of [
      `https://evil.example/assistant?c=${C}`,
      `https://app.nyxid.test.evil.example/assistant?c=${C}`,
      `http://app.nyxid.test/assistant?c=${C}`,
      `https://app.nyxid.test/assistant?c=${C}#top`,
      `https://me:pw@app.nyxid.test/assistant?c=${C}`,
      `other://assistant?c=${C}`,
      "nyxid://",
      "not a url",
      null,
      undefined,
    ])
      expect(notificationDeeplinkPath(link, ORIGINS, "nyxid")).toBeNull();
    expect(notificationDeeplinkPath(`https://app.nyxid.test/assistant?c=${C}`, ["", "nope"], "nyxid")).toBeNull();
  });
});

describe("createNotificationPushController", () => {
  it("navigates to the push's canonical page first and marks the row read only after the dispatch", async () => {
    const x = harness();
    const handled = x.controller.tapped({
      notificationId: "n1",
      deeplink: `https://app.nyxid.test/assistant?agent=${AGENT}&details=channels`,
    });
    expect(handled).toBe(true);
    expect(x.opened).toEqual([`/assistant?agent=${AGENT}&details=channels@0`]);
    await flushMicrotasks();
    expect(x.requests).toEqual([
      { path: "/assistant/nyxagent/notifications/read", method: "POST", body: { ids: ["n1"] } },
    ]);
    expect(x.invalidated(x.summaryKey)).toBe(true);
    expect(x.invalidated(x.feedKey)).toBe(true);
    expect(x.invalidated(x.otherKey)).toBe(false);
    expect(x.controller.hasPending()).toBe(false);
  });

  it("holds a cold-start or signed-out tap until the navigator can take it, then routes and reads once", async () => {
    const x = harness({ ready: false });
    expect(x.controller.tapped({ notificationId: "n2", deeplink: `nyxid://assistant?c=${C}` })).toBe(true);
    expect(x.opened).toEqual([]);
    expect(x.requests).toEqual([]);
    expect(x.controller.hasPending()).toBe(true);
    expect(x.controller.flush()).toBeUndefined();
    x.state.ready = true;
    await x.controller.flush();
    expect(x.opened).toEqual([`/assistant?c=${C}@0`]);
    expect(x.requests.map((request) => request.body)).toEqual([{ ids: ["n2"] }]);
    expect(x.controller.flush()).toBeUndefined();
    expect(x.opened).toHaveLength(1);
    expect(x.requests).toHaveLength(1);
  });

  it("ignores foreign, malformed and missing deeplinks without reading anything", async () => {
    const x = harness();
    expect(x.controller.tapped({ notificationId: "n3", deeplink: `https://evil.example/assistant?c=${C}` })).toBe(false);
    expect(x.controller.tapped({ notificationId: "n3", deeplink: null })).toBe(false);
    expect(x.controller.tapped(null)).toBe(false);
    await flushMicrotasks();
    expect(x.opened).toEqual([]);
    expect(x.requests).toEqual([]);
    expect(x.controller.hasPending()).toBe(false);
  });

  it("a later tap replaces a pending one; a read failure still refreshes the feed", async () => {
    const x = harness({ ready: false });
    x.controller.tapped({ notificationId: "old", deeplink: `nyxid://assistant?c=${C}` });
    x.controller.tapped({ notificationId: "new", deeplink: `https://app.nyxid.test/assistant?c=${C}&card=card-1` });
    x.state.ready = true;
    x.setFailRead(true);
    await x.controller.flush();
    expect(x.opened).toEqual([`/assistant?c=${C}&card=card-1@0`]);
    expect(x.requests.map((request) => request.body)).toEqual([{ ids: ["new"] }]);
    expect(x.invalidated(x.summaryKey)).toBe(true);
  });

  it("a push arriving in the live runtime marks notification queries stale, refetching only in front and online", () => {
    const front = harness();
    expect(front.controller.received({ type: "chat_update", notification_id: "n", deeplink: "x" })).toBe(true);
    expect(front.invalidated(front.summaryKey)).toBe(true);
    expect(front.invalidated(front.otherKey)).toBe(false);
    const background = harness({ foreground: false });
    expect(background.controller.received({ type: "needs_decision", notification_id: "n" })).toBe(true);
    expect(background.invalidated(background.feedKey)).toBe(true);
    expect(background.requests).toEqual([]);
    const approval = harness();
    expect(approval.controller.received({ type: "approval_request", challenge_id: "c" })).toBe(false);
    expect(approval.invalidated(approval.summaryKey)).toBe(false);
    expect(approval.controller.received(null)).toBe(false);
  });
});
