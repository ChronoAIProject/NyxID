import { fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it } from "vitest";
import { useThemeStore } from "@/stores/theme-store";
import { DisplaySettings } from "./display-settings";

beforeEach(() => {
  localStorage.clear();
  useThemeStore.setState({ mode: "light", systemPrefersDark: false });
  useThemeStore.getState().resetDisplay();
});

describe("DisplaySettings", () => {
  it("switches theme, text size, density and motion", async () => {
    const user = userEvent.setup();
    render(<DisplaySettings />);
    await user.click(within(screen.getByRole("radiogroup", { name: "Theme" })).getByRole("radio", { name: "Dark" }));
    await user.click(within(screen.getByRole("radiogroup", { name: "Text size" })).getByRole("radio", { name: "Larger" }));
    await user.click(within(screen.getByRole("radiogroup", { name: "Density" })).getByRole("radio", { name: "Compact" }));
    await user.click(within(screen.getByRole("radiogroup", { name: "Motion" })).getByRole("radio", { name: "Reduce" }));
    expect(useThemeStore.getState()).toMatchObject({ mode: "dark", textScale: 1.25, density: 0.875, motion: "reduce" });
  });

  it("edits a color by hex, flags weak contrast, and resets it", async () => {
    const user = userEvent.setup();
    render(<DisplaySettings />);
    const field = screen.getByRole("textbox", { name: "Tertiary text hex value" });
    await user.clear(field);
    await user.type(field, "#DDDDDD");
    expect(useThemeStore.getState().customColors.light).toEqual({ "text-tertiary": "#DDDDDD" });
    const checks = screen.getByRole("list", { name: "Contrast checks" });
    expect(within(checks).getAllByText(/needs 4.5:1/)).toHaveLength(2);

    await user.click(screen.getByRole("button", { name: "Reset Tertiary text" }));
    expect(useThemeStore.getState().customColors.light).toEqual({});
    expect(field).toHaveValue("#5B5B63");
  });

  it("does not commit partial hex input and restores it on blur", async () => {
    const user = userEvent.setup();
    render(<DisplaySettings />);
    const field = screen.getByRole("textbox", { name: "Borders and dividers hex value" });
    await user.clear(field);
    await user.type(field, "#12");
    expect(field).toHaveAttribute("aria-invalid", "true");
    await user.tab();
    expect(field).toHaveValue("#C4C4CA");
    expect(useThemeStore.getState().customColors.light).toEqual({});
  });

  it("sets the sidebar mode and width", async () => {
    const user = userEvent.setup();
    render(<DisplaySettings />);
    await user.click(within(screen.getByRole("radiogroup", { name: "Main sidebar" })).getByRole("radio", { name: "Collapsed" }));
    fireEvent.change(screen.getByRole("slider", { name: "Assistant sidebar width" }), { target: { value: "280" } });
    expect(useThemeStore.getState()).toMatchObject({ sidebarMode: "collapsed", sidebarWidths: { assistant: 280 } });
    await user.click(screen.getByRole("button", { name: "Reset assistant sidebar width" }));
    expect(useThemeStore.getState().sidebarWidths.assistant).toBe(200);
  });

  it("edits the other theme and offers to preview it", async () => {
    const user = userEvent.setup();
    render(<DisplaySettings />);
    await user.click(within(screen.getByRole("radiogroup", { name: "Theme to edit" })).getByRole("radio", { name: "Dark" }));
    expect(screen.getByRole("list", { name: "dark theme colors" })).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Switch to dark theme to preview" }));
    expect(useThemeStore.getState().mode).toBe("dark");
  });
});
