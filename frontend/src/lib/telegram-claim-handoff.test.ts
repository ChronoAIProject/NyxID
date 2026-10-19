import { afterEach, beforeEach, expect, it, vi } from "vitest";
import {
  captureTelegramClaim,
  clearTelegramClaimHandoff,
  preserveTelegramClaimForLogin,
  readTelegramClaimHandoff,
} from "./telegram-claim-handoff";
const code = "ABCDE-FGHJK-LMNPQ-RSTUV";
beforeEach(() => {
  clearTelegramClaimHandoff();
  window.history.replaceState(null, "", "/");
});
afterEach(() => {
  clearTelegramClaimHandoff();
  vi.useRealTimers();
});
it("strips the claim before router/auth boot and keeps signed-in handoff out of storage", () => {
  window.history.replaceState(
    null,
    "",
    `/channel-bots?connect=telegram-new&claim=${code}`,
  );
  captureTelegramClaim();
  expect(window.location.href).not.toContain(code);
  expect(new URL(window.location.href).searchParams.get("claim_entry")).toBe(
    "true",
  );
  expect(readTelegramClaimHandoff()?.code).toBe(code);
  expect(JSON.stringify({ localStorage, sessionStorage })).not.toContain(code);
  expect(readTelegramClaimHandoff()?.code).toBe(code);
  clearTelegramClaimHandoff();
  expect(readTelegramClaimHandoff()).toBeNull();
});
it("uses sessionStorage only for login, consumes it on setup, and keeps return_to clean", () => {
  window.history.replaceState(
    null,
    "",
    `/channel-bots?connect=telegram-new&claim=${code}`,
  );
  captureTelegramClaim();
  preserveTelegramClaimForLogin();
  const cleanReturn = window.location.href;
  expect(JSON.stringify(sessionStorage)).toContain(code);
  expect(JSON.stringify(localStorage)).not.toContain(code);
  window.history.replaceState(
    null,
    "",
    `/login?return_to=${encodeURIComponent(cleanReturn)}`,
  );
  captureTelegramClaim();
  expect(window.location.href).not.toContain(code);
  expect(readTelegramClaimHandoff()?.code).toBe(code);
  clearTelegramClaimHandoff();
  expect(JSON.stringify(sessionStorage)).not.toContain(code);
});
it("rejects expired login handoff and strips malformed codes", () => {
  vi.useFakeTimers();
  window.history.replaceState(
    null,
    "",
    `/channel-bots?connect=telegram-new&claim=${code}`,
  );
  captureTelegramClaim();
  preserveTelegramClaimForLogin();
  vi.advanceTimersByTime(15 * 60_000 + 1);
  expect(readTelegramClaimHandoff()).toBeNull();
  window.history.replaceState(
    null,
    "",
    `/channel-bots?connect=telegram-new&claim=${"x".repeat(100)}`,
  );
  captureTelegramClaim();
  expect(new URL(window.location.href).searchParams.has("claim")).toBe(false);
  expect(readTelegramClaimHandoff()).toBeNull();
  expect(sessionStorage.length).toBe(0);
});
