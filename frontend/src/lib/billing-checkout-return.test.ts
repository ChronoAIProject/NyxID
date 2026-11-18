import { beforeEach, afterEach, expect, it, vi } from "vitest";
import { consumeBillingCheckoutReturn, rememberBillingCheckout } from "./billing-checkout-return";
const actor = "11111111-1111-4111-8111-111111111111";
const org = "22222222-2222-4222-8222-222222222222";
beforeEach(() => { sessionStorage.clear(); vi.useFakeTimers(); });
afterEach(() => vi.useRealTimers());
it("restores org selection once after checkout, including assistant panel", () => {
  for (const presentation of ["page", "panel"] as const) {
    rememberBillingCheckout(actor, org, presentation);
    expect(consumeBillingCheckoutReturn(undefined, {})).toBeUndefined();
    expect(consumeBillingCheckoutReturn(actor, {})).toEqual(presentation === "page"
      ? { to: "/billing", search: { ownerId: org } }
      : { to: "/assistant", search: { panel: "billing", panelOwnerId: org } });
    expect(consumeBillingCheckoutReturn(actor, {})).toBeUndefined();
  }
});
it("never changes explicit navigation or another person's selection", () => {
  rememberBillingCheckout(actor, org, "page");
  expect(consumeBillingCheckoutReturn(actor, { tab: "usage" })).toBeUndefined();
  rememberBillingCheckout(actor, org, "page");
  expect(consumeBillingCheckoutReturn(org, {})).toBeUndefined();
  expect(consumeBillingCheckoutReturn(actor, {})).toBeUndefined();
});
it("expires metadata after two hours and leaves ordinary personal return unchanged", () => {
  rememberBillingCheckout(actor, org, "page");
  vi.advanceTimersByTime(2 * 60 * 60 * 1000);
  expect(consumeBillingCheckoutReturn(actor, {})).toBeUndefined();
  rememberBillingCheckout(actor, undefined, "page");
  expect(consumeBillingCheckoutReturn(actor, {})).toBeUndefined();
});
it("requires a signed-in actor for org checkout without storing payment credentials", () => {
  expect(() => rememberBillingCheckout(undefined, org, "page")).toThrow("Sign in again");
  rememberBillingCheckout(actor, org, "page");
  expect(JSON.parse(sessionStorage.getItem(sessionStorage.key(0)!)!)).toEqual({
    actorId: actor, ownerId: org, presentation: "page", expiresAt: Date.now() + 7200000,
  });
});
