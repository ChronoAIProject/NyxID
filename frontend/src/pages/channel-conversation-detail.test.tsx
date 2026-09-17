import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ChannelConversationItem } from "@/types/channels";
import { InitiatedMessageSettings } from "./channel-conversation-detail";

const { update, send } = vi.hoisted(() => ({ update: vi.fn(), send: vi.fn() }));
vi.mock("@/hooks/use-channel-conversations", () => ({
  useUpdateChannelConversation: () => ({
    mutateAsync: update,
    isPending: false,
  }),
  useSendChannelMessage: () => ({ mutateAsync: send, isPending: false }),
  useChannelConversation: vi.fn(),
}));
vi.mock("@/components/layout/dashboard-layout", () => ({
  useBreadcrumbLabel: vi.fn(),
}));
vi.mock("sonner", () => ({ toast: { success: vi.fn() } }));

const conversation: ChannelConversationItem = {
  id: "conversation",
  channel_bot_id: "bot",
  platform: "telegram",
  platform_conversation_id: "123",
  platform_conversation_type: "private",
  platform_sender_id: null,
  agent_api_key_id: "agent",
  default_agent: false,
  is_active: true,
  allow_agent_initiated: false,
  capabilities: { media: { inbound: ["image", "file"], outbound: ["image"] },
    initiated_send: true,
    reply_to: true,
    thread: true,
    edit: false,
  },
  last_message_at: null,
  created_at: "2026-09-15T00:00:00Z",
  updated_at: "2026-09-15T00:00:00Z",
};

beforeEach(() => {
  vi.clearAllMocks();
  update.mockResolvedValue(conversation);
  send.mockResolvedValue({ message_id: "message" });
});

describe("agent-initiated messaging controls", () => {
  it("requires an explicit saved opt-in before test-send", async () => {
    render(<InitiatedMessageSettings conversation={conversation} />);
    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
    expect(
      screen.getByRole("button", { name: "Send test message" }),
    ).toBeDisabled();
    fireEvent.click(
      screen.getByRole("switch", { name: "Allow unprompted messages" }),
    );
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Save" })).toBeEnabled(),
    );
    expect(update).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() =>
      expect(update).toHaveBeenCalledWith({
        id: "conversation",
        allow_agent_initiated: true,
      }),
    );
    expect(send).not.toHaveBeenCalled();
  });

  it("explains unsupported transports and disables the toggle", () => {
    render(
      <InitiatedMessageSettings
        conversation={{
          ...conversation,
          capabilities: { ...conversation.capabilities, initiated_send: false },
        }}
      />,
    );
    expect(screen.getByRole("switch")).toBeDisabled();
    expect(
      screen.getByText(
        "This platform does not support agent-initiated messages.",
      ),
    ).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "Send test message" }),
    ).not.toBeInTheDocument();
  });

  it("disables wildcard routes even if the adapter supports sends", () => {
    render(
      <InitiatedMessageSettings
        conversation={{ ...conversation, platform_conversation_id: "*" }}
      />,
    );
    expect(screen.getByRole("switch")).toBeDisabled();
    expect(screen.getByText(/no specific chat address/)).toBeInTheDocument();
  });

  it("submits an opted-in test message with a delivery key", async () => {
    render(
      <InitiatedMessageSettings
        conversation={{ ...conversation, allow_agent_initiated: true }}
      />,
    );
    fireEvent.change(screen.getByLabelText("Send test message"), {
      target: { value: "hello" },
    });
    await waitFor(() =>
      expect(
        screen.getByRole("button", { name: "Send test message" }),
      ).toBeEnabled(),
    );
    fireEvent.click(screen.getByRole("button", { name: "Send test message" }));
    await waitFor(() =>
      expect(send).toHaveBeenCalledWith({
        conversation_id: "conversation",
        message: { text: "hello" },
        idempotency_key: expect.any(String),
      }),
    );
  });
});
