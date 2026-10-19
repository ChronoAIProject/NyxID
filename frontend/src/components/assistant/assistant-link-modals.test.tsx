import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { AssistantLinkModalHost } from "./assistant-link-modals";

const account = vi.hoisted(() => ({ open: vi.fn() }));
vi.mock("@/hooks/use-account-panel", () => ({ useAccountPanel: () => account }));
beforeEach(() => account.open.mockReset());

vi.mock("@/components/connect-link/connect-link-content", () => ({
  ConnectLinkContent: ({
    token,
    embedded,
    redirectOnTerminal,
  }: {
    readonly token: string;
    readonly embedded?: boolean;
    readonly redirectOnTerminal?: boolean;
  }) => (
    <div
      data-testid="connect-modal-content"
      data-embedded={String(embedded)}
      data-redirect-on-terminal={String(redirectOnTerminal)}
    >
      Connecting {token}
    </div>
  ),
}));

vi.mock("@/components/channels/channel-bot-setup", () => ({
  ChannelBotSetup: ({
    onOpenChange,
    defaultPlatform,
    defaultLabel,
    defaultOrgId,
    stayInPlace,
  }: {
    readonly onOpenChange: (open: boolean) => void;
    readonly defaultPlatform?: string;
    readonly defaultLabel?: string;
    readonly defaultOrgId: string | null;
    readonly stayInPlace?: boolean;
  }) => (
    <div
      data-testid="channel-modal-content"
      data-label={defaultLabel}
      data-org-id={defaultOrgId ?? ""}
      data-stay-in-place={String(stayInPlace)}
    >
      Setting up {defaultPlatform}
      <button type="button" onClick={() => onOpenChange(false)}>
        Close setup
      </button>
    </div>
  ),
}));

describe("AssistantLinkModalHost", () => {
  it("opens hosted connector links in place and leaves ordinary links alone", async () => {
    const user = userEvent.setup();
    render(
      <AssistantLinkModalHost>
        <a href="/connect/nyx_clk_test">Connect GitHub</a>
        <a href="https://example.com/docs">Docs</a>
      </AssistantLinkModalHost>,
    );

    await user.click(screen.getByRole("link", { name: "Connect GitHub" }));
    const modal = screen.getByTestId("connect-modal-content");
    expect(modal).toHaveTextContent("nyx_clk_test");
    expect(modal).toHaveAttribute("data-embedded", "true");
    expect(modal).toHaveAttribute("data-redirect-on-terminal", "false");
    expect(
      screen.getByRole("link", { name: "Docs", hidden: true }),
    ).toHaveAttribute("href", "https://example.com/docs");
  });

  it("reuses the channel bot setup dialog and closes it through its existing contract", async () => {
    const user = userEvent.setup();
    render(
      <AssistantLinkModalHost>
        <a href="/channel-bots/connect/telegram?label=Support&target_org_id=org-123">
          Set up bot
        </a>
      </AssistantLinkModalHost>,
    );

    await user.click(screen.getByRole("link", { name: "Set up bot" }));
    const modal = screen.getByTestId("channel-modal-content");
    expect(modal).toHaveTextContent("telegram");
    expect(modal).toHaveAttribute("data-label", "Support");
    expect(modal).toHaveAttribute("data-org-id", "org-123");
    expect(modal).toHaveAttribute("data-stay-in-place", "true");
    await user.click(screen.getByRole("button", { name: "Close setup" }));
    expect(
      screen.queryByTestId("channel-modal-content"),
    ).not.toBeInTheDocument();
  });
});

it("opens same-origin account links on the current chat and leaves modified clicks alone", async () => {
  const user = userEvent.setup();
  render(<AssistantLinkModalHost><a href="/assistant?panel=settings&panelTab=security">Security settings</a></AssistantLinkModalHost>);
  const link = screen.getByRole("link", { name: "Security settings" });
  await user.keyboard("{Control>}");
  await user.click(link);
  await user.keyboard("{/Control}");
  expect(account.open).not.toHaveBeenCalled();
  await user.click(link);
  expect(account.open).toHaveBeenCalledWith("settings", { panelTab: "security" }, link);
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
});
