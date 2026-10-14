import { beforeEach, expect, it } from "vitest";
import { pendingSetupRequest } from "./platform-onboarding";
beforeEach(() => sessionStorage.clear());
it("retries lost creation responses across refresh without retaining facts", () => {
  const first = pendingSetupRequest("actor-a", "restaurant-starter");
  expect(pendingSetupRequest("actor-a", "restaurant-starter").id).toBe(
    first.id,
  );
  first.clear();
  expect(pendingSetupRequest("actor-a", "restaurant-starter").id).not.toBe(
    first.id,
  );
});
it("separates account/template identities and protects successors", () => {
  const a = pendingSetupRequest("a", "restaurant-starter");
  expect(pendingSetupRequest("b", "restaurant-starter").id).not.toBe(a.id);
  expect(pendingSetupRequest("a", "telegram-assistant").id).not.toBe(a.id);
  sessionStorage.setItem(
    "nyxid:onboarding:create:a:restaurant-starter",
    "successor",
  );
  a.clear();
  expect(
    sessionStorage.getItem("nyxid:onboarding:create:a:restaurant-starter"),
  ).toBe("successor");
});
