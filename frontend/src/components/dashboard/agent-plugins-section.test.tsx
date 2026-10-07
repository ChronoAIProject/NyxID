import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import type { ReactNode } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { AgentPluginsSection } from "./agent-plugins-section";

const { toastFns, copyToClipboard } = vi.hoisted(() => ({
  toastFns: { success: vi.fn(), error: vi.fn() },
  copyToClipboard: vi.fn(),
}));

vi.mock("sonner", () => ({ toast: toastFns }));

vi.mock("@/lib/utils", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/lib/utils")>()),
  copyToClipboard,
}));

vi.mock("@tanstack/react-router", () => ({
  Link: ({ children, to }: { children: ReactNode; to: string }) => (
    <a href={to}>{children}</a>
  ),
}));

describe("AgentPluginsSection", () => {
  beforeEach(() => {
    vi.clearAllMocks();
  });

  it("shows the Claude Code and Codex install commands", () => {
    render(<AgentPluginsSection />);

    expect(
      screen.getByRole("heading", { name: "Agent plugins" }),
    ).toBeInTheDocument();
    expect(
      screen.getByText(/claude mcp login plugin:nyxid:nyxid/),
    ).toBeInTheDocument();
    expect(screen.getByText(/codex mcp login nyxid/)).toBeInTheDocument();
  });

  it("copies the selected command and confirms", async () => {
    copyToClipboard.mockResolvedValueOnce(undefined);
    render(<AgentPluginsSection />);

    await userEvent.click(
      screen.getByRole("button", {
        name: "Copy Claude Code plugin install command",
      }),
    );

    expect(copyToClipboard).toHaveBeenCalledWith(
      "claude plugin marketplace add ChronoAIProject/NyxID\nclaude plugin install nyxid@nyxid\nclaude mcp login plugin:nyxid:nyxid",
    );
    expect(toastFns.success).toHaveBeenCalledWith(
      "Claude Code install command copied",
    );
  });

  it("reports a failed copy", async () => {
    copyToClipboard.mockRejectedValueOnce(new Error("denied"));
    render(<AgentPluginsSection />);

    await userEvent.click(
      screen.getByRole("button", { name: "Copy Codex plugin install command" }),
    );

    expect(toastFns.error).toHaveBeenCalledWith(
      "Failed to copy install command",
    );
  });
});
