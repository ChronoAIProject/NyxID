import { useState } from "react";
import { act, render, screen, within, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useAuthStore } from "@/stores/auth-store";
import { CommandPalette } from "./command-palette";
import { useApplyTheme } from "@/hooks/use-theme";
import { useThemeStore } from "@/stores/theme-store";

const { navigate } = vi.hoisted(() => ({ navigate: vi.fn() }));

vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => navigate,
}));

function Palette({ initiallyOpen = true }: { initiallyOpen?: boolean }) {
  const [open, setOpen] = useState(initiallyOpen);
  return <CommandPalette open={open} onOpenChange={setOpen} />;
}

function Shell({ onSubmit }: { onSubmit: () => void }) {
  useApplyTheme();
  const [open, setOpen] = useState(false);
  return (
    <form onSubmit={(event) => { event.preventDefault(); onSubmit(); }}>
      <button type="button" onClick={() => setOpen(true)}>Open search</button>
      <input aria-label="Background input" />
      <button type="submit">Background submit</button>
      <CommandPalette open={open} onOpenChange={setOpen} />
    </form>
  );
}

function expectThemeSurface(element: HTMLElement | null) {
  expect(element).toHaveClass("bg-popover", "shadow-lg", "border-hairline");
}

function expectNoHardcodedBackground(container: HTMLElement) {
  for (const element of container.querySelectorAll("[class]")) {
    expect(element.getAttribute("class")).not.toContain("bg-[#");
  }
}

beforeEach(() => {
  navigate.mockClear();
  useAuthStore.setState({ user: null });
});

describe("CommandPalette", () => {
  it("opens with themed input and result surfaces, focus, and the existing scrim and key hint", () => {
    const { container } = render(<Palette />);
    const input = screen.getByRole("combobox");
    const firstResult = screen.getByRole("option", { name: "Bot Setups" });

    expect(input).toHaveFocus();
    expect(input).toHaveAttribute("aria-activedescendant", firstResult.id);
    expect(firstResult).toHaveAttribute("aria-selected", "true");
    expect(input).toHaveAttribute(
      "placeholder",
      "Search pages, actions, or keywords...",
    );
    expectThemeSurface(input.parentElement);
    expectThemeSurface(firstResult.parentElement);
    expectNoHardcodedBackground(screen.getByRole("dialog"));
    expect(firstResult).toHaveClass("border-foreground", "bg-overlay-strong", "text-foreground");
    expect(firstResult.querySelector("svg")).toHaveClass("text-current");
    expect(screen.getByRole("button", { name: "Close search" })).toHaveClass("bg-overlay", "text-muted-foreground");
    expect(document.querySelector("[data-command-overlay]")).toHaveClass("bg-black/70");
    expect(container).toBeEmptyDOMElement(); // The modal uses the shared Radix portal.
  });

  it("uses the same theme surface for an empty search and restores results when cleared", async () => {
    const user = userEvent.setup();
    render(<Palette />);
    const input = screen.getByRole("combobox");

    await user.type(input, "no-such-page-2718");

    const emptyState = screen.getByText("No results for “no-such-page-2718”");
    expectThemeSurface(emptyState);
    expect(emptyState).toHaveRole("status");
    expect(input).not.toHaveAttribute("aria-activedescendant");
    expectThemeSurface(input.parentElement);
    expectNoHardcodedBackground(screen.getByRole("dialog"));
    expect(screen.queryByRole("option", { name: "Bot Setups" })).not.toBeInTheDocument();
    await user.keyboard("{Enter}");
    expect(navigate).not.toHaveBeenCalled();
    expect(input).toBeInTheDocument();

    await user.clear(input);

    expect(emptyState).not.toBeInTheDocument();
    const firstResult = screen.getByRole("option", { name: "Bot Setups" });
    expect(firstResult).toHaveClass("bg-overlay-strong");
    expectThemeSurface(firstResult.parentElement);
  });

  it("keeps keyboard selection and navigates to the selected result with its search parameters", async () => {
    const user = userEvent.setup();
    render(<Palette />);
    await user.type(screen.getByRole("combobox"), "machines");
    const machines = screen.getByRole("option", { name: "Machines" });
    const savedLogins = screen.getByRole("option", { name: "Saved logins" });

    expect(machines).toHaveClass("bg-overlay-strong");
    await user.keyboard("{ArrowDown}");
    expect(savedLogins).toHaveClass("border-foreground", "bg-overlay-strong", "text-foreground");
    expect(savedLogins).toHaveAttribute("aria-selected", "true");
    expect(machines).toHaveAttribute("aria-selected", "false");
    expect(machines).toHaveClass("hover:bg-overlay", "text-muted-foreground");
    await user.keyboard("{ArrowUp}");
    expect(machines).toHaveClass("bg-overlay-strong");
    await user.keyboard("{ArrowDown}{Enter}");

    expect(navigate).toHaveBeenCalledExactlyOnceWith({
      to: "/assistant/machines",
      search: { tab: "logins" },
    });
    expect(screen.queryByRole("combobox")).not.toBeInTheDocument();
  });

  it("keeps hover selection and mouse navigation", async () => {
    const user = userEvent.setup();
    render(<Palette />);
    const firstResult = screen.getByRole("option", { name: "Bot Setups" });
    const dashboard = screen.getByRole("option", { name: "Dashboard" });

    expect(dashboard).toHaveClass("hover:bg-overlay");
    await user.hover(dashboard);
    expect(dashboard).toHaveClass("border-foreground", "bg-overlay-strong", "text-foreground");
    expect(dashboard.querySelector("svg")).toHaveClass("text-current");
    expect(firstResult).not.toHaveClass("bg-overlay-strong");
    await user.click(dashboard);

    expect(navigate).toHaveBeenCalledExactlyOnceWith({ to: "/dashboard", search: {} });
    expect(screen.queryByRole("combobox")).not.toBeInTheDocument();
  });

  it("keeps the Action hint readable when its row is selected by pointer or keyboard", async () => {
    const user = userEvent.setup();
    render(<Palette />);
    await user.type(screen.getByRole("combobox"), "key");
    const createKey = screen.getByRole("option", { name: "Create API key Action" });
    const actionHint = within(createKey).getByText("Action");

    await user.hover(createKey);
    expect(createKey).toHaveClass("bg-overlay-strong");
    // Tertiary text falls below 4.5:1 on the dark selected-row overlay.
    expect(actionHint).toHaveClass("text-muted-foreground");
    expect(actionHint).not.toHaveClass("text-text-tertiary");

    await user.keyboard("{ArrowUp}");
    expect(createKey).not.toHaveClass("bg-overlay-strong");
    expect(actionHint).toHaveClass("text-muted-foreground");
    await user.keyboard("{ArrowDown}");
    expect(createKey).toHaveClass("bg-overlay-strong");
    expect(actionHint).toHaveClass("text-muted-foreground");
    await user.keyboard("{Enter}");

    expect(navigate).toHaveBeenCalledExactlyOnceWith({
      to: "/keys",
      search: { tab: "nyxid", action: "create-key" },
    });
    expect(screen.queryByRole("combobox")).not.toBeInTheDocument();
  });

  it("closes on Escape and clears the query before reopening", async () => {
    const user = userEvent.setup();
    render(<Palette />);
    await user.type(screen.getByRole("combobox"), "no-such-page-2718");
    await user.keyboard("{Escape}");

    expect(screen.queryByRole("combobox")).not.toBeInTheDocument();
    expect(navigate).not.toHaveBeenCalled();
    await user.keyboard("/");

    expect(screen.getByRole("combobox")).toHaveValue("");
    expect(screen.getByRole("option", { name: "Bot Setups" })).toHaveClass("bg-overlay-strong");
  });

  it("renders nothing while closed and opens from the search shortcut", async () => {
    const user = userEvent.setup();
    const { container } = render(<Palette initiallyOpen={false} />);

    expect(container).toBeEmptyDOMElement();
    await user.keyboard("/");

    expect(screen.getByRole("combobox")).toHaveFocus();
    await user.click(screen.getByRole("button", { name: "Close search" }));
    expect(container).toBeEmptyDOMElement();
  });

  it.each(["light", "dark"] as const)("contains Tab focus and restores the opener in the %s theme, with populated and empty results", async (mode) => {
    useThemeStore.setState({ mode });
    const user = userEvent.setup();
    const submit = vi.fn();
    render(<Shell onSubmit={submit} />);
    const opener = screen.getByRole("button", { name: "Open search" });

    for (const query of ["machines", "no-such-page-2718"]) {
      await user.click(opener);
      const input = screen.getByRole("combobox");
      const close = screen.getByRole("button", { name: "Close search" });
      expect(input).toHaveFocus();
      expect(input).toHaveValue("");
      expect(input.parentElement).toHaveClass("focus-within:border-foreground", "focus-within:ring-foreground");
      expect(close).toHaveAttribute("type", "button");
      expect(close).toHaveClass("focus-visible:outline-foreground");
      await user.type(input, query);
      expect(screen.queryByRole("button", { name: "Background submit" })).not.toBeInTheDocument();
      await user.tab();
      expect(close).toHaveFocus();
      await user.tab();
      expect(input).toHaveFocus();
      await user.tab({ shift: true });
      expect(close).toHaveFocus();
      await user.tab({ shift: true });
      expect(input).toHaveFocus();
      await user.tab();
      // Enter on Close must close the dialog, never choose an active result.
      await user.keyboard("{Enter}");
      await waitFor(() => expect(opener).toHaveFocus());
      expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
      expect(navigate).not.toHaveBeenCalled();
      expect(submit).not.toHaveBeenCalled();
    }
  });

  it("keeps the query and selection through theme changes, then returns focus on Escape and scrim dismissal", async () => {
    useThemeStore.setState({ mode: "light" });
    const user = userEvent.setup();
    render(<Shell onSubmit={vi.fn()} />);
    const opener = screen.getByRole("button", { name: "Open search" });
    await user.click(opener);
    const input = screen.getByRole("combobox");
    await user.type(input, "machines");
    await user.keyboard("{ArrowDown}");
    const saved = screen.getByRole("option", { name: "Saved logins" });
    act(() => useThemeStore.setState({ mode: "dark" }));
    expect(document.documentElement).toHaveClass("theme-dark");
    expect(input).toHaveValue("machines");
    expect(input).toHaveFocus();
    expect(input).toHaveAttribute("aria-activedescendant", saved.id);
    expect(saved).toHaveClass("border-foreground", "bg-overlay-strong");
    await user.keyboard("{Escape}");
    await waitFor(() => expect(opener).toHaveFocus());

    await user.click(opener);
    expect(screen.getByRole("combobox")).toHaveValue("");
    await user.type(screen.getByRole("combobox"), "no-such-page-2718");
    await user.click(document.querySelector<HTMLElement>("[data-command-overlay]")!);
    await waitFor(() => expect(opener).toHaveFocus());
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(navigate).not.toHaveBeenCalled();
  });

  it("consumes empty-result Enter without submitting a background form or firing shell shortcuts", async () => {
    const user = userEvent.setup();
    const submit = vi.fn();
    const shellKey = vi.fn();
    window.addEventListener("keydown", shellKey);
    try {
      render(<Shell onSubmit={submit} />);
      const opener = screen.getByRole("button", { name: "Open search" });
      await user.click(opener);
      const input = screen.getByRole("combobox");
      await user.type(input, "no-such-page-2718");
      shellKey.mockClear();
      await user.keyboard("{ArrowDown}{ArrowUp}{Enter}");
      expect(input).toHaveFocus();
      expect(input).not.toHaveAttribute("aria-activedescendant");
      expect(submit).not.toHaveBeenCalled();
      expect(shellKey).not.toHaveBeenCalled();
      expect(navigate).not.toHaveBeenCalled();
      await user.keyboard("{Escape}");
      await waitFor(() => expect(opener).toHaveFocus());
    } finally {
      window.removeEventListener("keydown", shellKey);
    }
  });
});
