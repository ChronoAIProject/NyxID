import { describe, expect, it } from "vitest";
import { getVisibleAdminNav, getVisibleMainNav } from "./sidebar";

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

describe("getVisibleAdminNav", () => {
  it("shows only Usage to usage readers without admin read access", () => {
    expect(
      getVisibleAdminNav({
        is_admin: false,
        role: "user",
        capabilities: { read_admin_usage: true },
      }).map((item) => item.to),
    ).toEqual(["/admin/usage"]);
  });

  it("hides admin nav from plain users", () => {
    expect(getVisibleAdminNav({ is_admin: false, role: "user" })).toEqual([]);
    expect(getVisibleAdminNav(null)).toEqual([]);
  });

  it("keeps the full read-only set for operators", () => {
    const routes = getVisibleAdminNav({
      is_admin: false,
      role: "operator",
    }).map((item) => item.to);
    expect(routes).toContain("/admin/users");
    expect(routes).toContain("/admin/usage");
    expect(routes).not.toContain("/admin/platform-credentials");
  });
});

it("uses the same Tools capability rule for desktop and mobile navigation", () => {
  expect(
    getVisibleAdminNav({ is_admin: false, role: "operator" }).map((i) => i.to),
  ).not.toContain("/admin/tools");
  expect(
    getVisibleAdminNav({
      is_admin: false,
      role: "operator",
      capabilities: { manage_tools: true },
    }).map((i) => i.to),
  ).toContain("/admin/tools");
  expect(
    getVisibleAdminNav({
      is_admin: false,
      role: "operator",
      capabilities: { manage_catalog: true },
    }).map((i) => i.to),
  ).not.toContain("/admin/tools");
  expect(
    getVisibleAdminNav({
      is_admin: false,
      role: "operator",
      capabilities: { manage_catalog: true, manage_tools: true },
    }).map((i) => i.to),
  ).toContain("/admin/tools");
});
