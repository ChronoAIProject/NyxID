import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import { useThemeStore } from "@/stores/theme-store";
import { useApplyTheme } from "./use-theme";

beforeEach(() => {
  useThemeStore.setState({ mode: "light", systemPrefersDark: false });
  useThemeStore.getState().resetDisplay();
});

describe("useApplyTheme", () => {
  it.each([undefined, "assistant"] as const)("applies the rebased default on %s without changing spacing", (surface) => {
    const { unmount } = renderHook(() => useApplyTheme(surface));
    expect(document.documentElement.style.fontSize).toBe("85%");
    expect(document.documentElement.style.getPropertyValue("--spacing")).toBe("");
    unmount();
  });

  it.each([undefined, "assistant"] as const)("applies every display preference on %s to <html> and removes them on unmount", (surface) => {
    const root = document.documentElement;
    const { unmount } = renderHook(() => useApplyTheme(surface));
    act(() => {
      const s = useThemeStore.getState();
      s.setTextScale(1.25);
      s.setDensity(1.37);
      s.setMotion("reduce");
      s.setCustomColor("light", "border", "#123456");
    });

    expect(root.classList.contains("theme-light")).toBe(true);
    expect(root.classList.contains("assistant-display")).toBe(surface === "assistant");
    expect(root.style.fontSize).toBe("106.25%");
    expect(root.style.getPropertyValue("--spacing")).toBe("5.48px");
    expect(root.style.getPropertyValue("--display-density")).toBe("1.37");
    expect(root.classList.contains("motion-reduce")).toBe(true);
    expect(document.getElementById("nyxid-custom-colors")?.textContent).toContain("--color-border: #123456;");

    unmount();
    expect(root.classList.contains("theme-light")).toBe(false);
    expect(root.classList.contains("assistant-display")).toBe(false);
    expect(root.style.fontSize).toBe("");
    expect(root.style.getPropertyValue("--spacing")).toBe("");
    expect(root.style.getPropertyValue("--display-density")).toBe("");
    expect(root.classList.contains("motion-reduce")).toBe(false);
    expect(document.getElementById("nyxid-custom-colors")).toBeNull();
  });
});
