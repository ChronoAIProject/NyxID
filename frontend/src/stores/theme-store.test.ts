import { describe, it, expect, beforeEach } from "vitest";
import { useThemeStore, resolveTheme } from "./theme-store";

const STORAGE_KEY = "nyxid.theme";

/**
 * The toggle must actually persist `mode` — `useApplyDashboardTheme` reads it
 * at boot to set the `<html>` class before first paint, so a non-persisted
 * choice would silently revert on reload (the same failure mode the
 * consent-store test guards against).
 */
describe("resolveTheme", () => {
  it("follows the OS when mode is system", () => {
    expect(resolveTheme("system", true)).toBe("dark");
    expect(resolveTheme("system", false)).toBe("light");
  });

  it("honours an explicit mode regardless of the OS", () => {
    expect(resolveTheme("light", true)).toBe("light");
    expect(resolveTheme("dark", false)).toBe("dark");
  });
});

describe("useThemeStore", () => {
  beforeEach(() => {
    localStorage.clear();
    useThemeStore.setState({ mode: "system", systemPrefersDark: true });
  });

  it("defaults to follow-system", () => {
    expect(useThemeStore.getState().mode).toBe("system");
  });

  it("persists an explicit mode to localStorage", () => {
    useThemeStore.getState().setMode("light");
    expect(useThemeStore.getState().mode).toBe("light");
    expect(localStorage.getItem(STORAGE_KEY)).toContain("light");
  });

  it("toggle flips from the currently-resolved theme to its opposite", () => {
    // system + OS-dark resolves to dark → first toggle lands on light.
    useThemeStore.setState({ mode: "system", systemPrefersDark: true });
    useThemeStore.getState().toggle();
    expect(useThemeStore.getState().mode).toBe("light");
    useThemeStore.getState().toggle();
    expect(useThemeStore.getState().mode).toBe("dark");
  });

  it("toggle off system respects the live OS preference", () => {
    // system + OS-light resolves to light → first toggle lands on dark.
    useThemeStore.setState({ mode: "system", systemPrefersDark: false });
    useThemeStore.getState().toggle();
    expect(useThemeStore.getState().mode).toBe("dark");
  });
});
