import { afterEach, beforeEach, expect, it, vi } from "vitest";
import {
  captureOnboardingLink,
  clearOnboardingLink,
  readOnboardingLink,
  onboardingConfirmationUrl,
} from "./platform-onboarding-handoff";

beforeEach(() => {
  clearOnboardingLink();
  window.history.replaceState(null, "", "/");
});
afterEach(() => {
  vi.restoreAllMocks();
  vi.useRealTimers();
  clearOnboardingLink();
});
it.each(["#code=ABCD1234", "?code=ABCD1234"])(
  "cleans %s before storage and preserves the handoff through login",
  (suffix) => {
    window.history.replaceState(null, "", `/onboarding/link${suffix}`);
    const original = sessionStorage.setItem.bind(sessionStorage);
    vi.spyOn(sessionStorage, "setItem").mockImplementation((key, value) => {
      expect(location.href).not.toContain("ABCD1234");
      original(key, value);
    });
    captureOnboardingLink();
    expect(readOnboardingLink()?.code).toBe("ABCD1234");
    window.history.replaceState(
      null,
      "",
      "/login?return_to=%2Fonboarding%2Flink",
    );
    expect(readOnboardingLink()?.code).toBe("ABCD1234");
  },
);
it.each([
  "#code=bad",
  "?code=ABCD1234#code=ABCD1234",
  "#code=ABCD1234&code=EFGH5678",
])("rejects malformed or ambiguous requests: %s", (suffix) => {
  window.history.replaceState(null, "", `/onboarding/link${suffix}`);
  captureOnboardingLink();
  expect(location.href).not.toContain("code=");
  expect(readOnboardingLink()).toBeUndefined();
});
it("expires requests and survives blocked tab storage without persisting a browser proof", () => {
  vi.useFakeTimers();
  vi.spyOn(sessionStorage, "setItem").mockImplementation(() => {
    throw new Error("blocked");
  });
  window.history.replaceState(null, "", "/onboarding/link#code=ABCD1234");
  captureOnboardingLink();
  expect(readOnboardingLink()?.code).toBe("ABCD1234");
  expect(location.hash).toBe("");
  vi.advanceTimersByTime(600001);
  expect(readOnboardingLink()).toBeUndefined();
});
it("only creates purpose-specific Telegram return links from a fresh confirmation", () => {
  const proof = `O-${"A".repeat(32)}`;
  expect(onboardingConfirmationUrl(`/confirm ${proof}`, "manager_bot")).toBe(
    `https://t.me/manager_bot?start=nyxonboard_${proof}`,
  );
  for (const command of [
    "/confirm ABCD1234",
    "/confirm P-ABCD-1234",
    `/confirm ${proof} extra`,
  ])
    expect(onboardingConfirmationUrl(command, "manager_bot")).toBeUndefined();
  expect(
    onboardingConfirmationUrl(`/confirm ${proof}`, "evil/path"),
  ).toBeUndefined();
});
