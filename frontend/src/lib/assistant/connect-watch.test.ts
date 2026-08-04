import { afterEach, describe, expect, it, vi } from "vitest";
import {
  CONNECT_WATCH_ACTIVITY_GRACE_MS,
  CONNECT_WATCH_BASE_MS,
  CONNECT_WATCH_FAST_INTERVAL_MS,
  CONNECT_WATCH_FAST_WINDOW_MS,
  CONNECT_WATCH_MAX_MS,
  CONNECT_WATCH_SLOW_INTERVAL_MS,
  connectWatchDeadline,
  connectWatchInterval,
  getLastChatActivityAt,
  markChatActivity,
  resetChatActivityForTests,
  subscribeChatActivity,
} from "./connect-watch";

const START = 1_000_000;

afterEach(() => {
  resetChatActivityForTests();
  vi.restoreAllMocks();
});

describe("connectWatchDeadline", () => {
  it("uses the base window when the user has not touched the chat", () => {
    expect(connectWatchDeadline(START, 0)).toBe(START + CONNECT_WATCH_BASE_MS);
  });

  it("ignores activity that predates the watch", () => {
    expect(connectWatchDeadline(START, START - 60_000)).toBe(
      START + CONNECT_WATCH_BASE_MS,
    );
  });

  it("never shortens the base window for activity early in it", () => {
    // A message sent one second after the handoff must not pull the deadline
    // in to +5 min; the base window is a floor, not a target.
    expect(connectWatchDeadline(START, START + 1_000)).toBe(
      START + CONNECT_WATCH_BASE_MS,
    );
  });

  it("extends past the base window while the user stays active", () => {
    const lateActivity = START + CONNECT_WATCH_BASE_MS - 1_000;
    expect(connectWatchDeadline(START, lateActivity)).toBe(
      lateActivity + CONNECT_WATCH_ACTIVITY_GRACE_MS,
    );
  });

  it("caps extensions at the hard ceiling", () => {
    const veryLateActivity = START + CONNECT_WATCH_MAX_MS;
    expect(connectWatchDeadline(START, veryLateActivity)).toBe(
      START + CONNECT_WATCH_MAX_MS,
    );
  });
});

describe("connectWatchInterval", () => {
  it("polls fast while the user is plausibly still on the provider page", () => {
    expect(connectWatchInterval(START, START)).toBe(
      CONNECT_WATCH_FAST_INTERVAL_MS,
    );
    expect(
      connectWatchInterval(START, START + CONNECT_WATCH_FAST_WINDOW_MS),
    ).toBe(CONNECT_WATCH_FAST_INTERVAL_MS);
  });

  it("backs off to the background cadence afterwards", () => {
    expect(
      connectWatchInterval(START, START + CONNECT_WATCH_FAST_WINDOW_MS + 1),
    ).toBe(CONNECT_WATCH_SLOW_INTERVAL_MS);
  });
});

describe("chat activity signal", () => {
  it("records the latest activity and notifies subscribers", () => {
    const listener = vi.fn();
    const unsubscribe = subscribeChatActivity(listener);

    markChatActivity(START);
    expect(getLastChatActivityAt()).toBe(START);
    expect(listener).toHaveBeenCalledTimes(1);

    markChatActivity(START + 5_000);
    expect(getLastChatActivityAt()).toBe(START + 5_000);
    expect(listener).toHaveBeenCalledTimes(2);

    unsubscribe();
    markChatActivity(START + 10_000);
    expect(listener).toHaveBeenCalledTimes(2);
  });

  it("never moves backwards, so a stale mark cannot shorten a watch", () => {
    markChatActivity(START);
    markChatActivity(START - 60_000);
    expect(getLastChatActivityAt()).toBe(START);
  });
});
