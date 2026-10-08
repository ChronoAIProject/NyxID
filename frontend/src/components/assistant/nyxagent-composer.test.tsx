import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { NyxAgentComposer } from "./nyxagent-composer";
import { nyxAgentTransport } from "@/lib/assistant/nyxagent-transport";
import { nyxAgentConversationSchema } from "@/schemas/assistant-nyxagent";
import { ApiError } from "@/lib/api-client";
import { useAssistantDraftStore } from "@/stores/assistant-draft-store";
import { ChatMessageBubble } from "./chat-message";

const conversation = nyxAgentConversationSchema.parse({
  id: `nyxa-${"1".repeat(32)}`, title: "Thread", model: "nyxagent/chat", created_at: "2026-10-01T00:00:00Z",
  last_message_at: "2026-10-01T00:00:00Z", message_count: 1, context_reset_at: null,
  active_turn: { turn_id: "turn-1", started_at: "2026-10-01T00:00:00Z", steering_allowed: true, response_id: "resp_1", stop_requested: false },
});
const accepted = { client_request_id: "request", outcome: "applied" as const, code: null };
const props = () => ({
  conversation, active: true, sending: true, ownerUserId: "owner", draftKey: "conv:one",
  scope: { kind: "conversations" as const, id: conversation.id },
  onSend: vi.fn().mockResolvedValue(undefined), onStop: vi.fn().mockResolvedValue(undefined),
});
function mount(initial = props()) {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  const view = render(<QueryClientProvider client={client}><NyxAgentComposer {...initial} /></QueryClientProvider>);
  return { ...view, props: initial, rerenderProps: (next: typeof initial) => view.rerender(<QueryClientProvider client={client}><NyxAgentComposer {...next} /></QueryClientProvider>) };
}
async function send(text = "Focus on the latest results") {
  await waitFor(() => expect(screen.getByRole("textbox")).toBeEnabled());
  fireEvent.change(screen.getByRole("textbox"), { target: { value: text } });
  fireEvent.click(screen.getByRole("button", { name: "Send guidance" }));
}
function refused(status: number, code: string) {
  return new ApiError(status, { error: code, error_code: -1, message: code });
}
beforeEach(() => {
  useAssistantDraftStore.setState({ ownerUserId: null, drafts: {} });
  vi.spyOn(nyxAgentTransport, "capabilities").mockResolvedValue({ steer: { max_chars: 32000, max_per_turn: 20 } });
  vi.spyOn(nyxAgentTransport, "steer").mockResolvedValue(accepted);
});
afterEach(() => vi.restoreAllMocks());

describe("NyxAgent web steering", () => {
  it("sends guidance to the running turn while keeping Stop and disabling files", async () => {
    const view = mount(); await send();
    await screen.findByText("Guidance applied to this reply.");
    expect(nyxAgentTransport.steer).toHaveBeenCalledWith(conversation.id, "Focus on the latest results", "turn-1", expect.any(String));
    expect(view.props.onSend).not.toHaveBeenCalled();
    expect(screen.getByRole("textbox")).toHaveValue("");
    expect(screen.getByRole("button", { name: "Attach files" })).toBeDisabled();
    fireEvent.click(screen.getByRole("button", { name: "Stop assistant turn" }));
    expect(view.props.onStop).toHaveBeenCalledOnce();
  });
  it("keeps the old blocked composer when steering is unavailable", async () => {
    vi.mocked(nyxAgentTransport.capabilities).mockResolvedValue({ steer: null }); mount();
    await waitFor(() => expect(nyxAgentTransport.capabilities).toHaveBeenCalled());
    expect(screen.getByRole("textbox")).toBeDisabled();
    expect(screen.queryByRole("button", { name: "Send guidance" })).not.toBeInTheDocument();
    expect(screen.getByRole("button", { name: "Stop assistant turn" })).toBeEnabled();
  });
  it("blocks guidance once Stop is pending", async () => {
    mount({ ...props(), conversation: { ...conversation, active_turn: { ...conversation.active_turn!, stop_requested: true, steering_allowed: false } } });
    await waitFor(() => expect(nyxAgentTransport.capabilities).toHaveBeenCalled());
    expect(screen.getByRole("textbox")).toBeDisabled(); expect(nyxAgentTransport.steer).not.toHaveBeenCalled();
  });
  it("uses the same request ID when retrying a starting response", async () => {
    vi.mocked(nyxAgentTransport.steer).mockRejectedValueOnce(refused(409,"starting")); mount(); await send();
    await screen.findByText(/assistant is starting/);
    await waitFor(() => expect(screen.getByRole("textbox")).toHaveValue("Focus on the latest results"));
    fireEvent.click(screen.getByRole("button", { name: "Send guidance" }));
    await screen.findByText("Guidance applied to this reply.");
    const calls = vi.mocked(nyxAgentTransport.steer).mock.calls;
    expect(calls).toHaveLength(2); expect(calls[1]![3]).toBe(calls[0]![3]);
  });
  it.each([
    [409,"response_mismatch",/running response changed/], [409,"idempotency_conflict",/request ID was reused/],
    [429,"steer_limit",/guidance limit/], [400,"invalid_request",/text only/],
    [404,"not_found",/conversation was not found/], [409,"stop_pending",/assistant is stopping/],
  ] as const)("shows %s %s and retains the refused draft", async (status,code,text) => {
    vi.mocked(nyxAgentTransport.steer).mockRejectedValue(refused(status,code)); mount(); await send();
    await screen.findByText(text);
    await waitFor(() => expect(screen.getByRole("textbox")).toHaveValue("Focus on the latest results"));
    expect(nyxAgentTransport.steer).toHaveBeenCalledOnce();
  });
  it.each([refused(503,"steer_unavailable"), new TypeError("network error")])("marks uncertain delivery and never automatically resends", async (error) => {
    vi.mocked(nyxAgentTransport.steer).mockRejectedValue(error);
    const view = mount(); await send(); await screen.findByText(/may not have been applied/);
    expect(screen.getByRole("textbox")).toHaveValue("");
    view.rerenderProps({ ...view.props, active: false, sending: false, conversation: { ...conversation, active_turn: null } });
    expect(nyxAgentTransport.steer).toHaveBeenCalledOnce(); expect(view.props.onSend).not.toHaveBeenCalled();
    expect(screen.queryByText("Send as new message")).not.toBeInTheDocument();
  });
  it("offers a separate explicit new-message action after no_active_turn", async () => {
    vi.mocked(nyxAgentTransport.steer).mockRejectedValue(refused(409,"no_active_turn"));
    const view = mount(); await send();
    const button = await screen.findByRole("button", { name: "Send as new message" });
    expect(button).toBeDisabled(); expect(view.props.onSend).not.toHaveBeenCalled();
    view.rerenderProps({ ...view.props, active: false, sending: false, conversation: { ...conversation, active_turn: null } });
    await act(async () => fireEvent.click(button));
    expect(view.props.onSend).toHaveBeenCalledExactlyOnceWith("Focus on the latest results");
  });
  it("shows persisted steering outcome on its user message", () => {
    render(<ChatMessageBubble message={{ id: "steer", role: "user", content: "Guidance", timestamp: 1, status: "complete", steering: accepted }} />);
    expect(screen.getByRole("note")).toHaveTextContent("Steering · Guidance applied to this reply.");
  });
});
