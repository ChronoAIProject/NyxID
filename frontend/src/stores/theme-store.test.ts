import { describe, it, expect, beforeEach, vi } from "vitest";
import { useThemeStore, resolveTheme } from "./theme-store";

const STORAGE_KEY = "nyxid.theme";

/**
 * The toggle must actually persist `mode` — `useApplyTheme` reads it
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
    useThemeStore.getState().resetDisplay();
  });

  it("stores custom colors per theme and ignores non-hex input", () => {
    const { setCustomColor } = useThemeStore.getState();
    setCustomColor("light", "border", "#abcdef");
    setCustomColor("light", "input", "red; }");
    expect(useThemeStore.getState().customColors).toEqual({ light: { border: "#ABCDEF" }, dark: {} });
  });

  it("resets one color, then a whole theme, then every display preference", () => {
    const s = useThemeStore.getState();
    s.setCustomColor("dark", "border", "#111111");
    s.setCustomColor("dark", "input", "#222222");
    s.resetCustomColors("dark", "border");
    expect(useThemeStore.getState().customColors.dark).toEqual({ input: "#222222" });
    s.resetCustomColors("dark");
    expect(useThemeStore.getState().customColors.dark).toEqual({});
    s.setDensity(1.125);
    s.setMotion("reduce");
    s.setMode("light");
    s.resetDisplay();
    expect(useThemeStore.getState()).toMatchObject({ density: 1, motion: "system", textScale: 1, mode: "light" });
  });

  it("clamps sidebar widths to the supported range", () => {
    const { setSidebarWidth } = useThemeStore.getState();
    setSidebarWidth("dashboard", 9999);
    setSidebarWidth("assistant", 10);
    expect(useThemeStore.getState().sidebarWidths).toEqual({ dashboard: 360, assistant: 180 });
  });

  it("keeps a sidebar mode chosen before it moved into display settings", async () => {
    // A user from before this change: theme persisted, sidebar mode in its old key.
    localStorage.setItem(STORAGE_KEY, JSON.stringify({ state: { mode: "dark" }, version: 1 }));
    localStorage.setItem("nyxid:sidebar-mode", "collapsed");
    vi.resetModules();
    const fresh = await import("./theme-store");
    expect(fresh.useThemeStore.getState().sidebarMode).toBe("collapsed");
  });

  it("drops tampered persisted values on load", async () => {
    localStorage.setItem(
      STORAGE_KEY,
      JSON.stringify({
        state: {
          mode: "sepia",
          density: 9,
          motion: "wild",
          sidebarMode: "floating",
          sidebarWidths: { dashboard: "wide", assistant: 5000 },
          customColors: { light: { border: "#fff;}" , input: "#123456" } },
        },
        version: 1,
      }),
    );
    await useThemeStore.persist.rehydrate();
    expect(useThemeStore.getState()).toMatchObject({
      mode: "system",
      density: 1,
      motion: "system",
      sidebarWidths: { dashboard: 200, assistant: 360 },
      customColors: { light: { input: "#123456" }, dark: {} },
    });
  });

  it("persists the text size alongside the mode", () => {
    useThemeStore.getState().setTextScale(1.25);
    expect(useThemeStore.getState().textScale).toBe(1.25);
    expect(JSON.parse(localStorage.getItem(STORAGE_KEY) ?? "{}").state).toMatchObject({
      mode: "system",
      textScale: 1.25,
    });
  });

  it("ignores a persisted text size outside the supported scale", async () => {
    localStorage.setItem(STORAGE_KEY, JSON.stringify({ state: { mode: "dark", textScale: 3 }, version: 1 }));
    await useThemeStore.persist.rehydrate();
    expect(useThemeStore.getState().mode).toBe("dark");
    expect(useThemeStore.getState().textScale).toBe(1);
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
