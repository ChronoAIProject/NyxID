import { afterEach, beforeEach, expect, it, vi } from "vitest";
import {
  capturePersonalLink,
  clearPersonalLink,
  readPersonalLink,
  savePersonalChallenge,
  shortenPersonalLink,
} from "./platform-personal-handoff";

const token = `nyx_pli_${"a".repeat(64)}`;
beforeEach(() => {
  clearPersonalLink();
  const entries = new Map<string, string>();
  vi.stubGlobal("sessionStorage", {
    getItem: (key: string) => entries.get(key) ?? null,
    setItem: (key: string, value: string) => entries.set(key, value),
    removeItem: (key: string) => entries.delete(key),
  });
  window.history.replaceState(null, "", "/");
});
afterEach(() => {
  vi.restoreAllMocks();
  vi.useRealTimers();
  clearPersonalLink();
  vi.unstubAllGlobals();
});

it("scrubs the fragment before writing dedicated tab storage and retains it across signup navigation", () => {
  window.history.replaceState(null, "", `/nyxbot/link#intent_token=${token}`);
  const original = sessionStorage.setItem;
  const setItem = vi.spyOn(sessionStorage, "setItem");
  setItem.mockImplementation(function (this: Storage, key, value) {
    expect(location.hash).toBe("");
    expect(location.href).not.toContain(token);
    return original.call(this, key, value);
  });
  capturePersonalLink();
  expect(readPersonalLink()?.token).toBe(token);
  expect(setItem).toHaveBeenCalledWith(
    "nyxid:personal-link:v1",
    expect.any(String),
  );
  expect(localStorage.getItem("nyxid:personal-link:v1")).toBeNull();
  window.history.replaceState(null, "", "/register?return_to=%2Fnyxbot%2Flink");
  expect(readPersonalLink()?.token).toBe(token);
});

it("caps lifetime at 24 hours and shortens it using server expiry", () => {
  vi.useFakeTimers();
  window.history.replaceState(null, "", `/nyxbot/link#intent_token=${token}`);
  capturePersonalLink();
  expect(readPersonalLink()?.expiresAt).toBe(Date.now() + 86400000);
  shortenPersonalLink(new Date(Date.now() + 60000).toISOString());
  savePersonalChallenge(
    "challenge",
    "account",
    new Date(Date.now() + 10000).toISOString(),
  );
  expect(sessionStorage.getItem("nyxid:personal-link:v1")).not.toContain(
    "/confirm",
  );
  vi.advanceTimersByTime(60001);
  expect(readPersonalLink()).toBeNull();
  expect(sessionStorage.getItem("nyxid:personal-link:v1")).toBeNull();
});

it.each([
  "#intent_token=malformed",
  `#intent_token=${token}&extra=1`,
  "#unexpected=secret",
  `?intent_token=${token}`,
  `?intent_token=${token}#intent_token=${token}`,
])("cleans and rejects malformed or query credentials: %s", (suffix) => {
  window.history.replaceState(null, "", `/nyxbot/link${suffix}`);
  capturePersonalLink();
  expect(location.href).not.toContain(token);
  expect(location.hash).toBe("");
  expect(readPersonalLink()).toBeNull();
});

it("keeps the URL clean with blocked storage and supports the current tab in memory", () => {
  window.history.replaceState(null, "", `/nyxbot/link#intent_token=${token}`);
  vi.spyOn(sessionStorage, "setItem").mockImplementation(() => {
    throw new Error("blocked");
  });
  capturePersonalLink();
  expect(location.hash).toBe("");
  expect(readPersonalLink()?.token).toBe(token);
});
