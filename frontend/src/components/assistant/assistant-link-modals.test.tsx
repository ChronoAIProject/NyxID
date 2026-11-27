import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { AssistantLinkModalHost } from "./assistant-link-modals";
import { NyxBotHandoffContext } from "@/lib/assistant/nyxbot-handoff-context";
import { transitionAssistantIdentity } from "@/lib/assistant/identity";

const account = vi.hoisted(() => ({ open: vi.fn() }));
vi.mock("@/hooks/use-account-panel", () => ({ useAccountPanel: () => account }));
beforeEach(() => account.open.mockReset());

vi.mock("@/components/channels/channel-connect-link-content", () => ({
  ChannelConnectLinkContent: ({ token, embedded, onClose }: { token: string; embedded?: boolean; onClose: () => void }) =>
    <div data-testid="hosted-bot-content" data-embedded={String(embedded)}>{token}<button onClick={onClose}>Back to chat</button></div>,
}));
vi.mock("@/components/dashboard/api-key-create-dialog", () => ({
  ApiKeyCreateDialog: ({ externalOpen, onExternalOpenChange, onComplete }: { externalOpen: boolean; onExternalOpenChange: (open: boolean) => void; onComplete?: (id: string) => Promise<void> }) =>
    externalOpen ? <div data-testid="agent-key-content"><button onClick={() => onExternalOpenChange(false)}>Close key setup</button><button onClick={() => void onComplete?.("11111111-1111-4111-8111-111111111111")}>Finish key setup</button></div> : null,
}));

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
    initialTelegramRequestId,
    onFinish,
  }: {
    readonly onOpenChange: (open: boolean) => void;
    readonly defaultPlatform?: string;
    readonly defaultLabel?: string;
    readonly defaultOrgId: string | null;
    readonly stayInPlace?: boolean;
    readonly initialTelegramRequestId?: string;
    readonly onFinish?: (id: string) => Promise<void>;
  }) => (
    <div
      data-testid="channel-modal-content"
      data-label={defaultLabel}
      data-org-id={defaultOrgId ?? ""}
      data-stay-in-place={String(stayInPlace)}
      data-request-id={initialTelegramRequestId}
      data-can-continue={String(Boolean(onFinish))}
    >
      Setting up {defaultPlatform}
      <button type="button" onClick={() => onOpenChange(false)}>
        Close setup
      </button>
    </div>
  ),
}));

describe("AssistantLinkModalHost", () => {
  it("opens Telegram request recovery as a plain return flow without resuming an unrelated chat", async () => {
    const user = userEvent.setup();
    const complete = vi.fn();
    const id = "11111111-1111-4111-8111-111111111111";
    render(<NyxBotHandoffContext value={{ continue: vi.fn(), open: vi.fn(), complete }}><AssistantLinkModalHost><a href={`/channel-bots?connect=telegram-new&request_id=${id}`}>Recover Telegram</a></AssistantLinkModalHost></NyxBotHandoffContext>);
    await user.click(screen.getByRole("link", { name: "Recover Telegram" }));
    const form = screen.getByTestId("channel-modal-content");
    expect(form).toHaveAttribute("data-request-id", id);
    expect(form).toHaveAttribute("data-can-continue", "false");
    await user.click(screen.getByRole("button", { name: "Close setup" }));
    expect(complete).not.toHaveBeenCalled();
  });

  it("provides completion only for a fresh channel setup started from the source chat", async () => {
    const user = userEvent.setup();
    render(<NyxBotHandoffContext value={{ continue: vi.fn(), open: vi.fn(), complete: vi.fn() }}><AssistantLinkModalHost><a href="/channel-bots/connect/telegram">New Telegram</a></AssistantLinkModalHost></NyxBotHandoffContext>);
    await user.click(screen.getByRole("link", { name: "New Telegram" }));
    expect(screen.getByTestId("channel-modal-content")).toHaveAttribute("data-can-continue", "true");
  });
  it("binds setup completion to the source chat that opened the form", async () => {
    const user = userEvent.setup();
    const original = vi.fn().mockResolvedValue(undefined);
    const next = vi.fn().mockResolvedValue(undefined);
    const tree = (complete: (result: unknown) => Promise<void>) => <NyxBotHandoffContext value={{ continue: vi.fn(), open: vi.fn(), complete }}><AssistantLinkModalHost><a href="/keys?tab=nyxid&action=create-key">Key setup</a></AssistantLinkModalHost></NyxBotHandoffContext>;
    const view = render(tree(original));
    await user.click(screen.getByRole("link", { name: "Key setup" }));
    view.rerender(tree(next));
    await user.click(screen.getByRole("button", { name: "Finish key setup" }));
    await waitFor(() => expect(original).toHaveBeenCalledWith({ kind: "agent_key", id: "11111111-1111-4111-8111-111111111111" }));
    expect(next).not.toHaveBeenCalled();
  });

  it("unmounts the setup flow when the active account changes", async () => {
    const user = userEvent.setup();
    render(<AssistantLinkModalHost><a href="/keys?tab=nyxid&action=create-key">Key setup</a></AssistantLinkModalHost>);
    await user.click(screen.getByRole("link", { name: "Key setup" }));
    expect(screen.getByTestId("agent-key-content")).toBeInTheDocument();
    act(() => transitionAssistantIdentity(crypto.randomUUID()));
    expect(screen.queryByTestId("agent-key-content")).not.toBeInTheDocument();
  });
  it("returns to a source specialist conversation inside the app", async () => {
    const user = userEvent.setup();
    const open = vi.fn();
    const id = `nyxa-${"a".repeat(32)}`;
    render(<NyxBotHandoffContext value={{ continue: vi.fn(), open }}><AssistantLinkModalHost><a href={`/assistant?c=${id}`}>Specialist chat</a></AssistantLinkModalHost></NyxBotHandoffContext>);
    await user.click(screen.getByRole("link", { name: "Specialist chat" }));
    expect(open).toHaveBeenCalledWith(id);
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  });
  it("opens the hosted bot page over chat, preserves its token, and closes in place", async () => {
    const user = userEvent.setup();
    const location = window.location.href;
    render(<AssistantLinkModalHost><a href="/connect/bot/nyx_bcl_example">Create Telegram bot</a><p>Specialist conversation</p></AssistantLinkModalHost>);
    await user.click(screen.getByRole("link", { name: "Create Telegram bot" }));
    expect(screen.getByRole("dialog", { name: "Set up your channel bot" })).toBeVisible();
    expect(screen.getByTestId("hosted-bot-content")).toHaveAttribute("data-embedded", "true");
    expect(screen.getByTestId("hosted-bot-content")).toHaveTextContent("nyx_bcl_example");
    await user.click(screen.getByRole("button", { name: "Back to chat" }));
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
    expect(screen.getByText("Specialist conversation")).toBeVisible();
    expect(window.location.href).toBe(location);
  });

  it("opens Agent Key creation in place", async () => {
    const user = userEvent.setup();
    const location = window.location.href;
    render(<AssistantLinkModalHost><a href="/keys?tab=nyxid&action=create-key">Create an Agent Key</a></AssistantLinkModalHost>);
    await user.click(screen.getByRole("link", { name: "Create an Agent Key" }));
    expect(screen.getByTestId("agent-key-content")).toBeInTheDocument();
    await user.click(screen.getByRole("button", { name: "Close key setup" }));
    expect(screen.queryByTestId("agent-key-content")).not.toBeInTheDocument();
    expect(window.location.href).toBe(location);
  });
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
