import { describe, expect, it } from "vitest";
import { getVisibleMainNav } from "./sidebar";

describe("getVisibleMainNav", () => {
  it("hides Billing when the backend capability is missing or false", () => {
    expect(getVisibleMainNav(null).map((item) => item.to)).not.toContain(
      "/billing",
    );
    expect(
      getVisibleMainNav({
        capabilities: { billing_available: false },
      }).map((item) => item.to),
    ).not.toContain("/billing");
  });

  it("includes Billing when the backend marks billing available", () => {
    expect(
      getVisibleMainNav({
        capabilities: { billing_available: true },
      }).map((item) => item.to),
    ).toContain("/billing");
  });
});

it("keeps NyxBot workspace features out of Studio navigation", () => {
  const items = getVisibleMainNav(null);
  for (const label of ["Automations", "Saved logins", "Machines"]) {
    expect(items.map((item) => item.label)).not.toContain(label);
  }
  expect(items.map((item) => item.to)).toContain("/nodes");
});
