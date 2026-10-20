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
          density: "wide",
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

  it("persists independent text size and spacing alongside the mode", () => {
    useThemeStore.getState().setTextScale(1.17);
    useThemeStore.getState().setDensity(1.37);
    expect(useThemeStore.getState().textScale).toBe(1.17);
    expect(JSON.parse(localStorage.getItem(STORAGE_KEY) ?? "{}").state).toMatchObject({
      mode: "system",
      textScale: 1.17,
      density: 1.37,
    });
  });

  it.each([0.5, 1.17, 1.83, 2])("restores granular display multipliers of %s", async (scale) => {
    localStorage.setItem(STORAGE_KEY, JSON.stringify({ state: { mode: "dark", textScale: scale, density: scale }, version: 1 }));
    await useThemeStore.persist.rehydrate();
    expect(useThemeStore.getState()).toMatchObject({ textScale: scale, density: scale });
  });

  it("rounds old density presets to the nearest slider step", async () => {
    localStorage.setItem(STORAGE_KEY, JSON.stringify({ state: { density: 0.875 }, version: 1 }));
    await useThemeStore.persist.rehydrate();
    expect(useThemeStore.getState().density).toBe(0.88);
  });

  it("bounds display multipliers and defaults nonfinite values", () => {
    const { setTextScale, setDensity } = useThemeStore.getState();
    setTextScale(9);
    setDensity(0.1);
    expect(useThemeStore.getState()).toMatchObject({ textScale: 2, density: 0.5 });
    setTextScale(Number.NaN);
    setDensity(Number.POSITIVE_INFINITY);
    expect(useThemeStore.getState()).toMatchObject({ textScale: 1, density: 1 });
  });

  it("caps a previously saved larger text size at the new maximum", async () => {
    localStorage.setItem(STORAGE_KEY, JSON.stringify({ state: { mode: "dark", textScale: 2.5 }, version: 1 }));
    await useThemeStore.persist.rehydrate();
    expect(useThemeStore.getState().mode).toBe("dark");
    expect(useThemeStore.getState().textScale).toBe(2);
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
