import { useState } from "react";
import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { useAuthStore } from "@/stores/auth-store";
import { CommandPalette } from "./command-palette";

const { navigate } = vi.hoisted(() => ({ navigate: vi.fn() }));

vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => navigate,
}));

function Palette({ initiallyOpen = true }: { initiallyOpen?: boolean }) {
  const [open, setOpen] = useState(initiallyOpen);
  return <CommandPalette open={open} onOpenChange={setOpen} />;
}

function expectThemeSurface(element: HTMLElement | null) {
  expect(element).toHaveClass("bg-popover", "shadow-lg", "border-hairline");
}

function expectNoHardcodedBackground(container: HTMLElement) {
  for (const element of container.querySelectorAll("[class]")) {
    expect(element.className).not.toContain("bg-[#");
  }
}

beforeEach(() => {
  navigate.mockClear();
  useAuthStore.setState({ user: null });
});

describe("CommandPalette", () => {
  it("opens with themed input and result surfaces, focus, and the existing scrim and key hint", () => {
    const { container } = render(<Palette />);
    const input = screen.getByRole("textbox");
    const firstResult = screen.getByRole("button", { name: "Bot Setups" });

    expect(input).toHaveFocus();
    expect(input).toHaveAttribute(
      "placeholder",
      "Search pages, actions, or keywords...",
    );
    expectThemeSurface(input.parentElement);
    expectThemeSurface(firstResult.parentElement);
    expectNoHardcodedBackground(container);
    expect(firstResult).toHaveClass("bg-overlay-strong", "text-foreground");
    expect(firstResult.querySelector("svg")).toHaveClass("text-nyx-secondary-400");
    expect(screen.getByText("esc")).toHaveClass("bg-overlay", "text-text-tertiary");
    expect(screen.getByRole("button", { name: "Close search" })).toHaveClass("bg-black/70");
  });

  it("uses the same theme surface for an empty search and restores results when cleared", async () => {
    const user = userEvent.setup();
    const { container } = render(<Palette />);
    const input = screen.getByRole("textbox");

    await user.type(input, "no-such-page-2718");

    const emptyState = screen.getByText("No results for “no-such-page-2718”");
    expectThemeSurface(emptyState);
    expectThemeSurface(input.parentElement);
    expectNoHardcodedBackground(container);
    expect(screen.queryByRole("button", { name: "Bot Setups" })).not.toBeInTheDocument();
    await user.keyboard("{Enter}");
    expect(navigate).not.toHaveBeenCalled();
    expect(input).toBeInTheDocument();

    await user.clear(input);

    expect(emptyState).not.toBeInTheDocument();
    const firstResult = screen.getByRole("button", { name: "Bot Setups" });
    expect(firstResult).toHaveClass("bg-overlay-strong");
    expectThemeSurface(firstResult.parentElement);
  });

  it("keeps keyboard selection and navigates to the selected result with its search parameters", async () => {
    const user = userEvent.setup();
    render(<Palette />);
    await user.type(screen.getByRole("textbox"), "machines");
    const machines = screen.getByRole("button", { name: "Machines" });
    const savedLogins = screen.getByRole("button", { name: "Saved logins" });

    expect(machines).toHaveClass("bg-overlay-strong");
    await user.keyboard("{ArrowDown}");
    expect(savedLogins).toHaveClass("bg-overlay-strong", "text-foreground");
    expect(machines).toHaveClass("hover:bg-overlay", "text-muted-foreground");
    await user.keyboard("{ArrowUp}");
    expect(machines).toHaveClass("bg-overlay-strong");
    await user.keyboard("{ArrowDown}{Enter}");

    expect(navigate).toHaveBeenCalledExactlyOnceWith({
      to: "/assistant/machines",
      search: { tab: "logins" },
    });
    expect(screen.queryByRole("textbox")).not.toBeInTheDocument();
  });

  it("keeps hover selection and mouse navigation", async () => {
    const user = userEvent.setup();
    render(<Palette />);
    const firstResult = screen.getByRole("button", { name: "Bot Setups" });
    const dashboard = screen.getByRole("button", { name: "Dashboard" });

    expect(dashboard).toHaveClass("hover:bg-overlay");
    await user.hover(dashboard);
    expect(dashboard).toHaveClass("bg-overlay-strong", "text-foreground");
    expect(dashboard.querySelector("svg")).toHaveClass("text-nyx-secondary-400");
    expect(firstResult).not.toHaveClass("bg-overlay-strong");
    await user.click(dashboard);

    expect(navigate).toHaveBeenCalledExactlyOnceWith({ to: "/dashboard", search: {} });
    expect(screen.queryByRole("textbox")).not.toBeInTheDocument();
  });

  it("keeps the Action hint readable when its row is selected by pointer or keyboard", async () => {
    const user = userEvent.setup();
    render(<Palette />);
    await user.type(screen.getByRole("textbox"), "key");
    const createKey = screen.getByRole("button", { name: "Create API key Action" });
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
    expect(screen.queryByRole("textbox")).not.toBeInTheDocument();
  });

  it("closes on Escape and clears the query before reopening", async () => {
    const user = userEvent.setup();
    render(<Palette />);
    await user.type(screen.getByRole("textbox"), "no-such-page-2718");
    await user.keyboard("{Escape}");

    expect(screen.queryByRole("textbox")).not.toBeInTheDocument();
    expect(navigate).not.toHaveBeenCalled();
    await user.keyboard("/");

    expect(screen.getByRole("textbox")).toHaveValue("");
    expect(screen.getByRole("button", { name: "Bot Setups" })).toHaveClass("bg-overlay-strong");
  });

  it("renders nothing while closed and opens from the search shortcut", async () => {
    const user = userEvent.setup();
    const { container } = render(<Palette initiallyOpen={false} />);

    expect(container).toBeEmptyDOMElement();
    await user.keyboard("/");

    expect(screen.getByRole("textbox")).toHaveFocus();
    await user.click(screen.getByRole("button", { name: "Close search" }));
    expect(container).toBeEmptyDOMElement();
  });
});
