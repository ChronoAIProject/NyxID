import { act, renderHook } from "@testing-library/react";
import { beforeEach, describe, expect, it } from "vitest";
import { useThemeStore } from "@/stores/theme-store";
import { useApplyTheme } from "./use-theme";

beforeEach(() => {
  useThemeStore.setState({ mode: "light", systemPrefersDark: false });
  useThemeStore.getState().resetDisplay();
});

describe("useApplyTheme", () => {
  it("applies every display preference to <html> and removes them on unmount", () => {
    const root = document.documentElement;
    const { unmount } = renderHook(() => useApplyTheme());
    act(() => {
      const s = useThemeStore.getState();
      s.setTextScale(1.25);
      s.setDensity(0.875);
      s.setMotion("reduce");
      s.setCustomColor("light", "border", "#123456");
    });

    expect(root.classList.contains("theme-light")).toBe(true);
    expect(root.style.fontSize).toBe("125%");
    expect(root.style.getPropertyValue("--spacing")).toBe("3.5px");
    expect(root.classList.contains("motion-reduce")).toBe(true);
    expect(document.getElementById("nyxid-custom-colors")?.textContent).toContain("--color-border: #123456;");

    unmount();
    expect(root.classList.contains("theme-light")).toBe(false);
    expect(root.style.fontSize).toBe("");
    expect(root.style.getPropertyValue("--spacing")).toBe("");
    expect(root.classList.contains("motion-reduce")).toBe(false);
    expect(document.getElementById("nyxid-custom-colors")).toBeNull();
  });
});
