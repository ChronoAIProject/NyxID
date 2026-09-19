import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import type { ChannelConversationItem, ChannelMessageItem } from "@/types/channels";
import { InitiatedMessageSettings, MessageCard } from "./channel-conversation-detail";

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

const message: ChannelMessageItem = {
  channel_bot_id: "bot", conversation_id: "conversation", agent_api_key_id: "agent",
  id: "message", direction: "inbound", platform: "whatsapp", platform_message_id: "wamid.example",
  sender_platform_id: "123", sender_display_name: null, content_type: "text", callback_status: "delivered",
  reply_to_message_id: null, created_at: "2026-09-20T00:00:00Z",
};

it.each([ ["delivered", "Callback accepted"], ["pending", "Callback pending"], ["failed", "Callback failed"], ["timeout", "Callback timed out"] ] as const)(
  "shows inbound %s as %s", (status, label) => {
    render(<MessageCard message={{ ...message, callback_status: status }} />);
    expect(screen.getByText(label)).toBeInTheDocument();
    expect(screen.queryByText("Platform accepted")).not.toBeInTheDocument();
  },
);

it("labels an outbound ID as platform acceptance without claiming delivery", () => {
  render(<MessageCard message={{ ...message, direction: "outbound", callback_status: null }} />);
  expect(screen.getByText("Platform accepted")).toBeInTheDocument();
  expect(screen.getByText(/Acceptance does not confirm recipient delivery/)).toHaveTextContent("wamid.example");
  expect(screen.queryByText("Delivered")).not.toBeInTheDocument();
  expect(screen.queryByText("Callback accepted")).not.toBeInTheDocument();
});

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

const delivery = {
  status: "partial" as const, complete: false, expected_components: 2,
  recipient_only: false, failure_code: 10005,
  components: [{ platform_message_id: "wamid.part", status: "read" as const,
    sent_at: null, delivered_at: "2026-09-20T01:00:00Z", read_at: "2026-09-20T01:00:00Z",
    played_at: null, failed_at: null, error_codes: [] }],
};

it("keeps partial acceptance visible when the accepted component was read", () => {
  render(<MessageCard message={{ ...message, direction: "outbound", delivery }} />);
  expect(screen.getByText("Partially accepted")).toBeInTheDocument();
  expect(screen.getByText(/1 of 2 component IDs recorded/)).toBeInTheDocument();
  expect(screen.getByText(/Check the conversation before sending again/)).toBeInTheDocument();
  expect(screen.getByText(/Send error code \(NyxID\): 10005/)).toBeInTheDocument();
  expect(screen.queryByText("Platform accepted")).not.toBeInTheDocument();
  expect(screen.getByText("Component 1: Read")).toBeInTheDocument();
});

it("shows unknown acceptance without inventing an ID", () => {
  render(<MessageCard message={{ ...message, direction: "outbound", platform_message_id: null,
    delivery: { ...delivery, status: "unknown", components: [] } }} />);
  expect(screen.getByText("Acceptance unknown")).toBeInTheDocument();
  expect(screen.queryByText("Component receipts")).not.toBeInTheDocument();
});

it("shows complete delivery and numeric component failures separately", () => {
  const { rerender } = render(<MessageCard message={{ ...message, direction: "outbound",
    delivery: { ...delivery, status: "delivered", complete: true, expected_components: 1, failure_code: null } }} />);
  expect(screen.getByText("Delivered")).toBeInTheDocument();
  expect(screen.queryByText(/before sending again/)).not.toBeInTheDocument();
  rerender(<MessageCard message={{ ...message, direction: "outbound",
    delivery: { ...delivery, status: "failed", complete: true, failure_code: null,
      components: [{ ...delivery.components[0]!, status: "failed", read_at: null, delivered_at: null,
        failed_at: "2026-09-20T01:00:00Z", error_codes: [131026] }] } }} />);
  expect(screen.getByText("Delivery failed")).toBeInTheDocument();
  expect(screen.getByText("WhatsApp error codes: 131026")).toBeInTheDocument();
});

it("states the legacy final-component and group limitations", () => {
  const { rerender } = render(<MessageCard message={{ ...message, direction: "outbound",
    delivery: { ...delivery, status: "legacy_final_only", expected_components: null, failure_code: null, components: [] } }} />);
  expect(screen.getByText("Final part accepted")).toBeInTheDocument();
  expect(screen.getByText(/Delivery of the whole reply cannot be confirmed/)).toBeInTheDocument();
  rerender(<MessageCard message={{ ...message, direction: "outbound",
    delivery: { ...delivery, status: "recipient_only", recipient_only: true, complete: true, failure_code: null } }} />);
  expect(screen.getByText(/do not confirm delivery to every group participant/)).toBeInTheDocument();
  expect(screen.queryByText("Delivered")).not.toBeInTheDocument();
});
