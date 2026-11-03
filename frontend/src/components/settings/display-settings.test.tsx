import { fireEvent, render, renderHook, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useApplyTheme } from "@/hooks/use-theme";
import { useThemeStore } from "@/stores/theme-store";
import { DisplaySettings } from "./display-settings";

beforeEach(() => {
  localStorage.clear();
  useThemeStore.setState({ mode: "light", systemPrefersDark: false });
  useThemeStore.getState().resetDisplay();
  HTMLElement.prototype.setPointerCapture = vi.fn();
});

describe("DisplaySettings", () => {
  it("supports keyboard selection and focus within display radio groups", async () => {
    const user = userEvent.setup();
    render(<DisplaySettings />);
    const theme = within(screen.getByRole("radiogroup", { name: "Theme" }));
    theme.getByRole("radio", { name: "Light" }).focus();
    await user.keyboard("{ArrowRight}");
    expect(theme.getByRole("radio", { name: "Dark" })).toHaveFocus();
    expect(useThemeStore.getState().mode).toBe("dark");
    await user.keyboard("{ArrowRight}");
    expect(theme.getByRole("radio", { name: "System" })).toHaveFocus();
    expect(useThemeStore.getState().mode).toBe("system");
    await user.keyboard("{End}");
    expect(theme.getByRole("radio", { name: "Dark" })).toHaveFocus();
    await user.keyboard("{Home}");
    expect(theme.getByRole("radio", { name: "System" })).toHaveFocus();
    const motion = within(screen.getByRole("radiogroup", { name: "Motion" }));
    motion.getByRole("radio", { name: "Follow system" }).focus();
    await user.keyboard("{ArrowDown}");
    expect(motion.getByRole("radio", { name: "Reduce" })).toHaveFocus();
    expect(useThemeStore.getState().motion).toBe("reduce");
  });

  it("switches theme, text size, spacing and motion", async () => {
    const user = userEvent.setup();
    render(<DisplaySettings />);
    await user.click(within(screen.getByRole("radiogroup", { name: "Theme" })).getByRole("radio", { name: "Dark" }));
    fireEvent.change(screen.getByRole("slider", { name: "Text size" }), { target: { value: "1.17" } });
    fireEvent.change(screen.getByRole("slider", { name: "Spacing" }), { target: { value: "0.83" } });
    await user.click(within(screen.getByRole("radiogroup", { name: "Motion" })).getByRole("radio", { name: "Reduce" }));
    expect(useThemeStore.getState()).toMatchObject({ mode: "dark", textScale: 1.17, density: 0.83, motion: "reduce" });
  });

  it.each([
    { label: "Text size", field: "textScale", otherField: "density" },
    { label: "Spacing", field: "density", otherField: "textScale" },
  ] as const)("adjusts $label across the full range independently", ({ label, field, otherField }) => {
    useThemeStore.getState().setDensity(0.83);
    useThemeStore.getState().setTextScale(0.83);
    render(<DisplaySettings />);
    const slider = screen.getByRole("slider", { name: label });
    expect(slider).toHaveAttribute("min", "0.5");
    expect(slider).toHaveAttribute("max", "2");
    expect(slider).toHaveAttribute("step", "0.01");
    for (const value of ["0.5", "1.83", "2"]) {
      fireEvent.change(slider, { target: { value } });
      expect(slider).toHaveAttribute("aria-valuetext", `${value}×`);
      expect(useThemeStore.getState()[field]).toBe(Number(value));
    }
    expect(screen.queryByRole("button", { name: "Reset text size to 1×" })).not.toBeInTheDocument();
    expect(useThemeStore.getState()[otherField]).toBe(0.83);
  });

  it.each([
    { label: "Text size", field: "textScale", pointerType: "mouse" },
    { label: "Text size", field: "textScale", pointerType: "touch" },
    { label: "Spacing", field: "density", pointerType: "mouse" },
    { label: "Spacing", field: "density", pointerType: "touch" },
  ] as const)("applies and saves $label only on $pointerType release", ({ label, field, pointerType }) => {
    renderHook(() => useApplyTheme());
    render(<DisplaySettings />);
    const slider = screen.getByRole("slider", { name: label });
    const persisted = localStorage.getItem("nyxid.theme");
    const root = document.documentElement;

    fireEvent.pointerDown(slider, { button: 0, pointerId: 1, pointerType });
    for (const value of ["1.17", "1.83"]) {
      fireEvent.change(slider, { target: { value } });
      expect(slider).toHaveAttribute("aria-valuetext", `${value}×`);
      expect(useThemeStore.getState()[field]).toBe(1);
      expect(localStorage.getItem("nyxid.theme")).toBe(persisted);
      expect(root.style.fontSize).toBe("");
      expect(root.style.getPropertyValue("--spacing")).toBe("");
    }

    fireEvent.pointerUp(slider, { pointerId: 1, pointerType });
    expect(useThemeStore.getState()[field]).toBe(1.83);
    expect(JSON.parse(localStorage.getItem("nyxid.theme")!).state[field]).toBe(1.83);
    expect(root.style.fontSize).toBe(field === "textScale" ? "183%" : "");
    expect(root.style.getPropertyValue("--spacing")).toBe(field === "density" ? "7.32px" : "");
  });

  it.each(["Text size", "Spacing"])("discards a cancelled %s drag", (label) => {
    render(<DisplaySettings />);
    const slider = screen.getByRole("slider", { name: label });
    const persisted = localStorage.getItem("nyxid.theme");

    fireEvent.pointerDown(slider, { button: 0, pointerId: 1 });
    fireEvent.change(slider, { target: { value: "1.83" } });
    fireEvent.pointerCancel(slider, { pointerId: 1 });
    fireEvent.pointerUp(slider, { pointerId: 1 });

    expect(slider).toHaveValue("1");
    expect(slider).toHaveAttribute("aria-valuetext", "1×");
    expect(useThemeStore.getState()).toMatchObject({ textScale: 1, density: 1 });
    expect(localStorage.getItem("nyxid.theme")).toBe(persisted);
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

  it("previews and saves chat colors for the edited theme independently", async () => {
    const user = userEvent.setup();
    render(<DisplaySettings />);
    await user.click(within(screen.getByRole("radiogroup", { name: "Theme to edit" })).getByRole("radio", { name: "Dark" }));
    fireEvent.change(screen.getByLabelText("Chat bubble background color"), { target: { value: "#303036" } });
    fireEvent.change(screen.getByLabelText("Chat bubble text color"), { target: { value: "#ffffff" } });
    const preview = within(screen.getByRole("region", { name: "Chat appearance preview" }));
    expect(preview.getByText("Show me the services I have connected.")).toHaveStyle({ backgroundColor: "#303036", color: "#FFFFFF" });
    expect(useThemeStore.getState().mode).toBe("light");
    expect(useThemeStore.getState().customColors.light).toEqual({});
    expect(JSON.parse(localStorage.getItem("nyxid.theme")!).state.customColors.dark).toEqual({
      "chat-bubble": "#303036", "chat-bubble-foreground": "#FFFFFF",
    });
    fireEvent.change(screen.getByLabelText("Chat bubble text color"), { target: { value: "#303036" } });
    expect(within(screen.getByRole("list", { name: "Contrast checks" })).getByText("1.0:1 (needs 4.5:1)")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Reset dark colors" }));
    expect(useThemeStore.getState().customColors.dark).toEqual({});
    expect(preview.getByText("Show me the services I have connected.")).toHaveStyle({ backgroundColor: "#2C2C30", color: "#EFEFF1" });
  });
});
