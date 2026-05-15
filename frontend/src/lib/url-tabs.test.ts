import { describe, expect, it } from "vitest";
import {
  SETTINGS_TABS,
  KEYS_TABS,
  isValidTab,
  parseTab,
} from "./url-tabs";

describe("parseTab", () => {
  it("returns the value when it matches the allowlist", () => {
    expect(parseTab("security", SETTINGS_TABS, "profile")).toBe("security");
  });

  it("returns the fallback when the value is not in the allowlist", () => {
    expect(parseTab("not-a-tab", SETTINGS_TABS, "profile")).toBe("profile");
  });

  it("returns the fallback when the value is undefined", () => {
    expect(parseTab(undefined, SETTINGS_TABS, "profile")).toBe("profile");
  });

  it("returns the fallback when the value is a non-string", () => {
    expect(parseTab(42, KEYS_TABS, "services")).toBe("services");
    expect(parseTab(null, KEYS_TABS, "services")).toBe("services");
    expect(parseTab({}, KEYS_TABS, "services")).toBe("services");
  });

  it("narrows the return type to the allowlist literal", () => {
    const tab = parseTab("nyxid", KEYS_TABS, "services");
    // Type-level assertion: `tab` is `"services" | "nyxid"`. If a future
    // change widens the return type, this assignment will fail to compile.
    const _typed: (typeof KEYS_TABS)[number] = tab;
    void _typed;
    expect(tab).toBe("nyxid");
  });
});

describe("isValidTab", () => {
  it("returns true for valid values", () => {
    expect(isValidTab("profile", SETTINGS_TABS)).toBe(true);
  });

  it("returns false for invalid string values", () => {
    expect(isValidTab("nope", SETTINGS_TABS)).toBe(false);
  });

  it("returns false for non-string values", () => {
    expect(isValidTab(undefined, SETTINGS_TABS)).toBe(false);
    expect(isValidTab(0, SETTINGS_TABS)).toBe(false);
    expect(isValidTab(null, SETTINGS_TABS)).toBe(false);
  });
});
