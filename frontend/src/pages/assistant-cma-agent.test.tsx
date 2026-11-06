import { fireEvent, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import type { CmaTranscriptTurn } from "@/schemas/nyxbot-cma-agent";
import { AssistantCmaAgentPage } from "./assistant-cma-agent";
const mocks = vi.hoisted(() => ({
  response: undefined as string | undefined,
  enabled: true,
  agent: vi.fn(),
  task: vi.fn(),
  history: vi.fn(),
  snapshot: vi.fn(),
  stream: vi.fn(),
}));
vi.mock("@tanstack/react-router", () => ({
  useParams: () => ({ agentId: "local-agent" }),
  useSearch: () => ({ response: mocks.response }),
  Link: ({
    to,
    params,
    search,
    children,
  }: {
    to: string;
    params: Record<string, string>;
    search?: Record<string, string>;
    children: React.ReactNode;
  }) => (
    <a
      href={
        Object.entries(params).reduce(
          (path, [key, value]) => path.replace(`$${key}`, value),
          to,
        ) +
        (search && Object.keys(search).length
          ? `?${new URLSearchParams(search)}`
          : "")
      }
    >
      {children}
    </a>
  ),
}));
vi.mock("@/hooks/use-feature-flag", () => ({
  useFeature: () => mocks.enabled,
}));
vi.mock("@/hooks/use-nyxbot-cma-agents", () => ({
  useCmaAgent: mocks.agent,
  useCmaResponse: mocks.task,
  useCmaTranscript: mocks.history,
  useCmaEventSnapshot: mocks.snapshot,
  useCmaEventStream: mocks.stream,
}));
vi.mock("@/components/assistant/cma-desktop-panel", () => ({
  CmaDesktopPanel: ({ agentId }: { agentId: string }) => (
    <div>Desktop for {agentId}</div>
  ),
}));
const message = (sequence: number, role: string, content: string) => ({
  id: null,
  sequence,
  role,
  content,
  created_at: null,
});
const turns: CmaTranscriptTurn[] = [
  {
    turn_sequence: 10,
    started_at: null,
    status: "completed",
    messages: [
      message(9, "user", "Prepare the workspace"),
      message(12, "assistant", "Earlier setup reply"),
    ],
  },
  {
    turn_sequence: 20,
    started_at: null,
    status: "active",
    messages: [
      message(19, "user", "Open the example page"),
      message(22, "assistant", "The title is **Example Domain**."),
      message(23, "tool", "Raw tool output"),
    ],
  },
];
function history(value = turns) {
  return {
    data: { pages: [{ turns: value, messages: [], next_cursor: null }] },
    refetch: vi.fn(),
  };
}
function snapshot() {
  return {
    data: {
      pages: [
        {
          events: [
            {
              id: "start10",
              sequence: 10,
              type: "turn_started",
              response_id: "rsp-old",
              turn_sequence: 10,
            },
            {
              id: "start20",
              sequence: 20,
              type: "turn_started",
              response_id: "rsp-new",
              turn_sequence: 20,
            },
            {
              id: "wait24",
              sequence: 24,
              type: "status_changed",
              status: "waiting_approval",
            },
          ],
          through_cursor: "24",
          next_cursor: null,
        },
      ],
    },
    refetch: vi.fn(),
    fetchNextPage: vi.fn(),
  };
}
function linkedResponse(agentId = "agt-one") {
  mocks.response = "rsp-old";
  mocks.task.mockReturnValue({
    data: {
      id: "rsp-old",
      agent_id: agentId,
      status: "completed",
      output: [{ content: "Untrusted response output" }],
    },
    refetch: vi.fn(),
  });
}
beforeEach(() => {
  vi.clearAllMocks();
  mocks.response = undefined;
  mocks.enabled = true;
  mocks.agent.mockReturnValue({
    data: {
      id: "local-agent",
      cma_agent_id: "agt-one",
      label: "Review assistant",
      status: "working",
      sandbox_id: "local-sandbox",
      available: true,
    },
    refetch: vi.fn(),
  });
  mocks.task.mockReturnValue({ data: undefined, refetch: vi.fn() });
  mocks.history.mockReturnValue(history());
  mocks.snapshot.mockReturnValue(snapshot());
  mocks.stream.mockReturnValue({
    events: [],
    state: "connected",
    reconnect: vi.fn(),
  });
});
it("uses one Conversation view beside Desktop with requests shared across chats", () => {
  render(<AssistantCmaAgentPage />);
  expect(screen.getByRole("tab", { name: "Conversation" })).toBeInTheDocument();
  expect(screen.getByRole("tab", { name: "Desktop" })).toBeInTheDocument();
  expect(
    screen.queryByRole("button", { name: "Selected task" }),
  ).not.toBeInTheDocument();
  expect(
    screen.queryByRole("button", { name: "Agent history" }),
  ).not.toBeInTheDocument();
  expect(
    screen.getByText(/all NyxBot chats that use this assistant/),
  ).toBeInTheDocument();
  expect(
    screen.queryByRole("button", {
      name: /stop agent|delete agent|create agent/i,
    }),
  ).not.toBeInTheDocument();
});
it("opens the newest conversation and keeps old replies and technical records folded", () => {
  render(<AssistantCmaAgentPage />);
  const latest = within(screen.getByRole("region", { name: "Latest request" }));
  expect(latest.getByText("Example Domain").tagName).toBe("STRONG");
  expect(latest.getByText("Work request")).toBeInTheDocument();
  expect(latest.getByText("Assistant reply")).toBeInTheDocument();
  expect(latest.getByText("Waiting for confirmation")).toBeInTheDocument();
  expect(screen.queryByText("Earlier setup reply")).not.toBeInTheDocument();
  expect(screen.queryByText("Raw tool output")).not.toBeInTheDocument();
  fireEvent.click(latest.getByRole("button", { name: /Execution details/ }));
  expect(latest.getByText("Raw tool output")).toBeInTheDocument();
  expect(latest.getByText("Status changed")).toBeInTheDocument();
  fireEvent.click(
    screen.getByRole("button", { name: /Prepare the workspace/ }),
  );
  expect(screen.getByText("Earlier setup reply")).toBeInTheDocument();
  expect(mocks.history).toHaveBeenCalledWith("local-agent", true);
  expect(mocks.stream).toHaveBeenCalledWith(
    "local-agent",
    "agt-one",
    undefined,
    true,
    "24",
  );
});
it("opens the exact earlier turn and offers a query-free latest link", () => {
  linkedResponse();
  render(<AssistantCmaAgentPage />);
  expect(
    screen.getByText(/You are viewing an earlier request/),
  ).toBeInTheDocument();
  const linked = within(screen.getByRole("region", { name: "Linked request" }));
  expect(linked.getByText("Earlier setup reply")).toBeInTheDocument();
  expect(linked.getByText("Completed")).toBeInTheDocument();
  expect(
    linked.queryByText("Waiting for confirmation"),
  ).not.toBeInTheDocument();
  expect(screen.queryByText("Example Domain")).not.toBeInTheDocument();
  expect(
    screen.queryByText("Untrusted response output"),
  ).not.toBeInTheDocument();
  expect(screen.getByRole("link", { name: "View latest" })).toHaveAttribute(
    "href",
    "/assistant/cma-agents/local-agent",
  );
});
it("does not select any turn when the exact response belongs to another assistant", () => {
  linkedResponse("agt-other");
  render(<AssistantCmaAgentPage />);
  expect(screen.getByRole("alert")).toHaveTextContent(
    /does not belong to this assistant/,
  );
  expect(
    screen.queryByRole("region", { name: "Linked request" }),
  ).not.toBeInTheDocument();
  expect(
    screen.queryByText("Untrusted response output"),
  ).not.toBeInTheDocument();
  expect(screen.queryByText("Example Domain")).not.toBeInTheDocument();
});
it("leaves an unbound old link unresolved instead of choosing the latest conversation", () => {
  linkedResponse();
  mocks.snapshot.mockReturnValue({
    data: { pages: [{ events: [], next_cursor: null }] },
  });
  render(<AssistantCmaAgentPage />);
  expect(
    screen.getByText(/could not be located in the available conversation/),
  ).toBeInTheDocument();
  expect(screen.queryByText("Earlier setup reply")).not.toBeInTheDocument();
  expect(screen.queryByText("Example Domain")).not.toBeInTheDocument();
});
it("keeps interrupted outcomes distinct from completed or cancelled", () => {
  mocks.history.mockReturnValue(
    history([
      {
        ...turns[1]!,
        status: "interrupted",
        messages: [message(19, "user", "Open the example page")],
      },
    ]),
  );
  render(<AssistantCmaAgentPage />);
  const latest = within(screen.getByRole("region", { name: "Latest request" }));
  expect(latest.getByText("Interrupted")).toBeInTheDocument();
  expect(latest.getByText(/request was interrupted/)).toBeInTheDocument();
  expect(
    latest.getByText("No assistant reply was published."),
  ).toBeInTheDocument();
  expect(latest.queryByText("Cancelled")).not.toBeInTheDocument();
  expect(latest.queryByText("Completed")).not.toBeInTheDocument();
});
it("preserves the entry being read on live arrivals and scrolls only when asked", () => {
  const scroll = vi.fn();
  const previous = HTMLElement.prototype.scrollIntoView;
  HTMLElement.prototype.scrollIntoView = scroll;
  try {
    const { rerender } = render(<AssistantCmaAgentPage />);
    mocks.history.mockReturnValue(
      history([
        ...turns,
        {
          turn_sequence: 30,
          started_at: null,
          status: "active",
          messages: [message(29, "user", "Read another page")],
        },
      ]),
    );
    rerender(<AssistantCmaAgentPage />);
    expect(screen.getByText("Example Domain")).toBeInTheDocument();
    const latest = within(
      screen.getByRole("region", { name: "Latest request" }),
    );
    expect(latest.getByRole("button")).toHaveAttribute(
      "aria-expanded",
      "false",
    );
    expect(scroll).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "View new activity" }));
    expect(
      latest.getByRole("button", { name: /Latest request/ }),
    ).toHaveAttribute("aria-expanded", "true");
    expect(scroll).toHaveBeenCalledTimes(1);
  } finally {
    HTMLElement.prototype.scrollIntoView = previous;
  }
});
it("does not reset a reader's expanded entries when more execution details load", () => {
  linkedResponse();
  const { rerender } = render(<AssistantCmaAgentPage />);
  fireEvent.click(
    screen.getByRole("button", { name: /Open the example page/ }),
  );
  mocks.snapshot.mockReturnValue({ ...snapshot(), isFetchingNextPage: true });
  rerender(<AssistantCmaAgentPage />);
  expect(screen.getByText("Example Domain")).toBeInTheDocument();
  expect(screen.getByText("Earlier setup reply")).toBeInTheDocument();
});
it("pauses conversation observation on Desktop and unmounts the viewer when returning", async () => {
  const user = userEvent.setup();
  render(<AssistantCmaAgentPage />);
  await user.click(screen.getByRole("tab", { name: "Desktop" }));
  expect(screen.getByText("Desktop for local-agent")).toBeInTheDocument();
  expect(mocks.history).toHaveBeenLastCalledWith("local-agent", false);
  await user.click(screen.getByRole("tab", { name: "Conversation" }));
  expect(screen.queryByText("Desktop for local-agent")).not.toBeInTheDocument();
  expect(mocks.history).toHaveBeenLastCalledWith("local-agent", true);
});
it("hides conversation and desktop on direct flag-off access", () => {
  mocks.enabled = false;
  render(<AssistantCmaAgentPage />);
  expect(screen.getByText(/not enabled for this account/i)).toBeInTheDocument();
  expect(screen.queryByText("Review assistant")).not.toBeInTheDocument();
  expect(mocks.history).toHaveBeenCalledWith("local-agent", false);
});
