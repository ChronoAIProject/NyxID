import {
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import type { CmaOperation } from "@/schemas/cma-operation";
import { CmaOperationCard } from "./cma-operation-card";

const mocks = vi.hoisted(() => ({
  agent: vi.fn(),
  task: vi.fn(),
  transcript: vi.fn(),
  snapshot: vi.fn(),
  stream: vi.fn(),
  desktop: vi.fn(),
}));
vi.mock("@/hooks/use-feature-flag", () => ({ useFeature: () => true }));
vi.mock("@/hooks/use-assistant-cma-agents", () => ({
  useCmaAgent: mocks.agent,
  useCmaResponse: mocks.task,
  useCmaTranscript: mocks.transcript,
  useCmaEventSnapshot: mocks.snapshot,
  useCmaEventStream: mocks.stream,
  useCmaDesktop: mocks.desktop,
}));
vi.mock("@tanstack/react-router", () => ({
  Link: ({
    to,
    params,
    children,
    ...props
  }: {
    to: string;
    params: Record<string, string>;
    children: React.ReactNode;
  }) => (
    <a
      {...props}
      href={Object.entries(params).reduce(
        (path, [key, value]) => path.replace(`$${key}`, value),
        to,
      )}
    >
      {children}
    </a>
  ),
}));

const agentId = "0dc85e4c-6aaf-47de-9df0-e07a69b8ee19";
const operation: CmaOperation = {
  id: "watch-1",
  turn_id: "turn-1",
  kind: "cma_response",
  operation_id: "resp-one",
  agent_ref_id: agentId,
  response_id: "resp-one",
  agent_id: "agt-one",
  idempotency_key: "shopping",
  status: "completed",
  phase: null,
  state: "delivered",
  reason: "completed",
  monitor_name: "NyxBot",
  created_at: "2026-11-06T06:42:00Z",
  checked_at: "2026-11-06T06:43:00Z",
};

beforeEach(() => {
  vi.clearAllMocks();
  mocks.agent.mockReturnValue({
    data: {
      id: agentId,
      cma_agent_id: "agt-one",
      label: "Practice assistant",
      status: "ready",
      available: true,
      sandbox_id: "sandbox-one",
    },
    refetch: vi.fn(),
  });
  mocks.task.mockReturnValue({
    data: { id: "resp-one", agent_id: "agt-one", status: "completed" },
    refetch: vi.fn(),
  });
  mocks.transcript.mockReturnValue({
    data: {
      pages: [
        {
          turns: [
            {
              turn_sequence: 1,
              started_at: null,
              status: "completed",
              messages: [
                {
                  sequence: 1,
                  role: "user",
                  content: "Calculate the shopping total",
                  created_at: null,
                },
                {
                  sequence: 2,
                  role: "assistant",
                  content: "The total is **103.00**.",
                  created_at: null,
                },
              ],
            },
          ],
        },
      ],
    },
    refetch: vi.fn(),
  });
  mocks.snapshot.mockReturnValue({
    data: {
      pages: [
        {
          events: [
            {
              id: "start1",
              sequence: 1,
              type: "turn_started",
              response_id: "resp-one",
              turn_sequence: 1,
            },
          ],
          through_cursor: "2",
          next_cursor: null,
        },
      ],
    },
    refetch: vi.fn(),
    fetchNextPage: vi.fn(),
  });
  mocks.stream.mockReturnValue({
    events: [],
    state: "connected",
    reconnect: vi.fn(),
  });
  mocks.desktop.mockReturnValue({
    data: { available: false, reason: "machine_not_included" },
    refetch: vi.fn(),
  });
});

function renderChat() {
  return render(
    <main>
      <div
        role="region"
        aria-label="NyxBot conversation"
        data-chat-transcript=""
      >
        <p>Original NyxBot reply</p>
        <CmaOperationCard operation={operation} />
      </div>
      <textarea
        data-composer-textbox=""
        aria-label="Message NyxBot"
        defaultValue="Keep my draft"
      />
    </main>,
  );
}

async function openPanel() {
  await userEvent.click(screen.getByRole("button", { name: "View execution" }));
  const panel = await screen.findByRole("dialog", { name: "Cloud assistant" });
  await within(panel).findByRole("heading", {
    name: "Practice assistant",
    level: 2,
  });
  return panel;
}

it("does not observe CMA until the card is opened", () => {
  renderChat();
  expect(mocks.agent).not.toHaveBeenCalled();
  expect(mocks.desktop).not.toHaveBeenCalled();
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
});

it("opens the exact agent and response inside the chat without navigation", async () => {
  renderChat();
  const url = window.location.href;
  const panel = await openPanel();
  expect(window.location.href).toBe(url);
  expect(mocks.agent).toHaveBeenCalledWith(agentId);
  expect(mocks.task).toHaveBeenCalledWith(agentId, "resp-one", true);
  expect(
    within(panel).getByRole("region", { name: "Linked request" }),
  ).toHaveTextContent("103.00");
  expect(
    within(panel).getByRole("link", { name: "Environment" }),
  ).toHaveAttribute("target", "_blank");
  expect(
    screen.queryByRole("link", { name: "View execution" }),
  ).not.toBeInTheDocument();
});

it("closes on Escape without remounting or scrolling the source chat", async () => {
  renderChat();
  const source = screen.getByRole("region", { name: "NyxBot conversation" });
  const trigger = screen.getByRole("button", { name: "View execution" });
  source.scrollTop = 120;
  await openPanel();
  await userEvent.keyboard("{Escape}");
  await waitFor(() =>
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument(),
  );
  expect(screen.getByRole("region", { name: "NyxBot conversation" })).toBe(
    source,
  );
  expect(source.scrollTop).toBe(120);
  expect(screen.getByRole("textbox", { name: "Message NyxBot" })).toHaveValue(
    "Keep my draft",
  );
  await waitFor(() => expect(trigger).toHaveFocus());
  expect(
    screen.queryByRole("region", { name: "Conversation messages" }),
  ).not.toBeInTheDocument();
});

it("preserves conversation position across Desktop and keeps Back to chat available", async () => {
  renderChat();
  const panel = await openPanel();
  const conversation = within(panel).getByRole("region", {
    name: "Conversation messages",
  });
  conversation.scrollTop = 85;
  await userEvent.click(within(panel).getByRole("tab", { name: "Desktop" }));
  expect(
    within(panel).getByText(/This environment has no desktop capability/),
  ).toBeInTheDocument();
  expect(
    within(panel).getByRole("button", { name: "Back to chat" }),
  ).toBeInTheDocument();
  expect(mocks.transcript).toHaveBeenLastCalledWith(agentId, false);
  await userEvent.click(
    within(panel).getByRole("tab", { name: "Conversation" }),
  );
  expect(
    within(panel).getByRole("region", { name: "Conversation messages" }),
  ).toBe(conversation);
  expect(conversation.scrollTop).toBe(85);
  expect(mocks.transcript).toHaveBeenLastCalledWith(agentId, true);
});

it("returns focus to the existing composer from Desktop without changing the draft", async () => {
  renderChat();
  const composer = screen.getByRole("textbox", { name: "Message NyxBot" });
  const panel = await openPanel();
  await userEvent.click(within(panel).getByRole("tab", { name: "Desktop" }));
  await userEvent.click(
    within(panel).getByRole("button", { name: "Back to chat" }),
  );
  await waitFor(() =>
    expect(screen.queryByRole("dialog")).not.toBeInTheDocument(),
  );
  await waitFor(() => expect(composer).toHaveFocus());
  expect(composer).toHaveValue("Keep my draft");
});

it("allows nested execution details to close without closing the assistant", async () => {
  renderChat();
  const panel = await openPanel();
  await userEvent.click(
    within(panel).getByRole("button", { name: /Execution details/ }),
  );
  expect(
    screen.getByRole("dialog", { name: "Execution details" }),
  ).toBeInTheDocument();
  await userEvent.keyboard("{Escape}");
  await waitFor(() =>
    expect(
      screen.queryByRole("dialog", { name: "Execution details" }),
    ).not.toBeInTheDocument(),
  );
  expect(screen.getByRole("dialog", { name: "Cloud assistant" })).toBe(panel);
});

it("expands the panel without remounting the conversation", async () => {
  renderChat();
  const panel = await openPanel();
  const conversation = within(panel).getByRole("region", {
    name: "Conversation messages",
  });
  fireEvent.click(within(panel).getByRole("button", { name: "Expand panel" }));
  expect(
    within(panel).getByRole("button", { name: "Restore panel width" }),
  ).toHaveAttribute("aria-pressed", "true");
  expect(
    within(panel).getByRole("region", { name: "Conversation messages" }),
  ).toBe(conversation);
});

it("removes the open panel when the source card leaves the chat", async () => {
  const view = renderChat();
  await openPanel();
  view.unmount();
  expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
  expect(
    screen.queryByRole("region", { name: "Conversation messages" }),
  ).not.toBeInTheDocument();
});
