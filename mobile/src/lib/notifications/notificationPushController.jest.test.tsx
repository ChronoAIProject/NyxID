import { QueryClient } from "@tanstack/react-query";
import { createNotificationsApi } from "../api/notificationsTransport";
import { NOTIFICATIONS } from "../../features/notifications/notificationKeys";
import { createNotificationPushController } from "./notificationPushController";
import { nativeNotificationUrl } from "./notificationDestination";

const ORIGINS = ["https://app.nyxid.test", "https://app.nyxid.dev"];
const C = `nyxa-${"a".repeat(32)}`;
const AGENT = "7c0f6a8e-2b2e-4a1a-9f1f-0d2c5a6b7c8d";
const clients: QueryClient[] = [];
afterEach(() => { for (const client of clients.splice(0)) client.clear(); });
function harness(failRead = false) {
  const requests: { path: string; method?: string; body?: unknown }[] = [];
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
  const invalidations = jest.spyOn(queryClient, "invalidateQueries");
  const state = { foreground: true, online: true };
  const controller = createNotificationPushController({
    api, queryClient,
    isForeground: () => state.foreground,
    isOnline: () => state.online,
  });
  const invalidated = (key: unknown[]) => queryClient.getQueryState(key)?.isInvalidated === true;
  return { controller, requests, invalidated, invalidations, state, summaryKey, feedKey, otherKey };
}

describe("native notification destination validation", () => {
  it("accepts first-party https links and the app scheme, nothing else", () => {
    expect(nativeNotificationUrl(`https://app.nyxid.test/assistant?c=${C}`, { scheme: "nyxid", trustedUrls: ORIGINS })).toBe(`nyxid://assistant?c=${C}`);
    expect(nativeNotificationUrl(`https://app.nyxid.dev/assistant?agent=${AGENT}&details=channels`, { scheme: "nyxid", trustedUrls: ORIGINS })).toBe(
      `nyxid://assistant?agent=${AGENT}&details=channels`,
    );
    expect(nativeNotificationUrl(`nyxid://assistant?c=${C}`, { scheme: "nyxid", trustedUrls: ORIGINS })).toBe(`nyxid://assistant?c=${C}`);
    expect(nativeNotificationUrl("nyxid://assistant/automations?agent=a", { scheme: "nyxid", trustedUrls: ORIGINS })).toBe("nyxid://assistant/automations?agent=a");
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
      expect(nativeNotificationUrl(link, { scheme: "nyxid", trustedUrls: ORIGINS })).toBeNull();
    expect(nativeNotificationUrl(`https://app.nyxid.test/assistant?c=${C}`, { scheme: "nyxid", trustedUrls: ["", "nope"] })).toBeNull();
  });
});


describe("notification push feed effects (navigation belongs to the app queue)", () => {
  for (const failRead of [false, true]) {
    it(`marks the dispatched row once and refreshes the feed, read failure=${failRead}`, async () => {
      const x = harness(failRead);
      await x.controller.navigated("n1");
      expect(x.requests).toEqual([
        { path: "/assistant/nyxagent/notifications/read", method: "POST", body: { ids: ["n1"] } },
      ]);
      expect(x.invalidated(x.summaryKey)).toBe(true);
      expect(x.invalidated(x.feedKey)).toBe(true);
      expect(x.invalidated(x.otherKey)).toBe(false);
    });
  }
  for (const [foreground, online] of [[true, true], [false, true], [true, false]]) {
    it(`refetches arrivals only in front and online (${foreground}, ${online})`, () => {
      const x = harness();
      Object.assign(x.state, { foreground, online });
      expect(x.controller.received({ type: "chat_update", notification_id: "n" })).toBe(true);
      expect(x.invalidations).toHaveBeenCalledWith({ queryKey: NOTIFICATIONS, refetchType: foreground && online ? "active" : "none" });
      expect(x.invalidated(x.summaryKey)).toBe(true);
      expect(x.invalidated(x.otherKey)).toBe(false);
      expect(x.requests).toEqual([]);
    });
  }
  it("leaves approval and malformed pushes alone", () => {
    const x = harness();
    expect(x.controller.received({ type: "approval_request", challenge_id: "c" })).toBe(false);
    expect(x.controller.received(null)).toBe(false);
    expect(x.invalidations).not.toHaveBeenCalled();
    expect(x.requests).toEqual([]);
  });
});
