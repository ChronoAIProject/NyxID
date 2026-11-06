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
it("shows every request and reply chronologically while keeping technical records out of the conversation", () => {
  mocks.history.mockReturnValue(history([...turns].reverse()));
  render(<AssistantCmaAgentPage />);
  const latest = within(screen.getByRole("region", { name: "Latest request" }));
  expect(latest.getByText("Example Domain").tagName).toBe("STRONG");
  expect(latest.getByText("Work request")).toBeInTheDocument();
  expect(latest.getAllByText("Review assistant")).toHaveLength(1);
  expect(latest.getByText("Waiting for confirmation")).toBeInTheDocument();
  expect(screen.getByText("Earlier setup reply")).toBeInTheDocument();
  expect(
    screen
      .getByText("Earlier setup reply")
      .compareDocumentPosition(screen.getByText("Open the example page")) &
      Node.DOCUMENT_POSITION_FOLLOWING,
  ).toBeTruthy();
  expect(
    screen.queryByRole("button", { name: /Latest request|Earlier request/ }),
  ).not.toBeInTheDocument();
  expect(screen.queryByText("Raw tool output")).not.toBeInTheDocument();
  fireEvent.click(latest.getByRole("button", { name: /Execution details/ }));
  const details = within(
    screen.getByRole("dialog", { name: "Execution details" }),
  );
  expect(details.getByText("Raw tool output")).toBeInTheDocument();
  expect(details.getByText("Status changed")).toBeInTheDocument();
  expect(mocks.history).toHaveBeenCalledWith("local-agent", true);
  expect(mocks.stream).toHaveBeenCalledWith(
    "local-agent",
    "agt-one",
    undefined,
    true,
    "24",
  );
});
it("marks the exact earlier request without hiding newer replies", () => {
  linkedResponse();
  render(<AssistantCmaAgentPage />);
  expect(
    screen.getByText(/Opened at the request from your chat/),
  ).toBeInTheDocument();
  const linked = within(screen.getByRole("region", { name: "Linked request" }));
  expect(linked.getByText("Earlier setup reply")).toBeInTheDocument();
  expect(linked.getByText("Completed")).toBeInTheDocument();
  expect(
    linked.queryByText("Waiting for confirmation"),
  ).not.toBeInTheDocument();
  expect(linked.getByText("From your chat")).toBeInTheDocument();
  expect(screen.getByText("Example Domain")).toBeInTheDocument();
  expect(
    screen.queryByText("Untrusted response output"),
  ).not.toBeInTheDocument();
  expect(
    screen.getByRole("button", { name: "Jump to latest" }),
  ).toBeInTheDocument();
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
  expect(screen.getByText("Example Domain")).toBeInTheDocument();
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
  expect(screen.getByText("Earlier setup reply")).toBeInTheDocument();
  expect(screen.getByText("Example Domain")).toBeInTheDocument();
  expect(screen.queryByText("From your chat")).not.toBeInTheDocument();
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
    scroll.mockClear();
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
    expect(latest.getByText("Read another page")).toBeInTheDocument();
    expect(scroll).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "View new activity" }));
    expect(scroll).toHaveBeenCalledTimes(1);
  } finally {
    HTMLElement.prototype.scrollIntoView = previous;
  }
});
it("keeps the execution details dialog open when more activity loads", () => {
  linkedResponse();
  const { rerender } = render(<AssistantCmaAgentPage />);
  fireEvent.click(
    within(screen.getByRole("region", { name: "Latest request" })).getByRole(
      "button",
      { name: /Execution details/ },
    ),
  );
  mocks.snapshot.mockReturnValue({ ...snapshot(), isFetchingNextPage: true });
  rerender(<AssistantCmaAgentPage />);
  expect(
    screen.getByRole("dialog", { name: "Execution details" }),
  ).toBeInTheDocument();
  expect(screen.getByText("Raw tool output")).toBeInTheDocument();
});

it("notifies about a new reply in the same request without scrolling the reader", () => {
  const scroll = vi.fn();
  const previous = HTMLElement.prototype.scrollIntoView;
  HTMLElement.prototype.scrollIntoView = scroll;
  try {
    linkedResponse();
    const { rerender } = render(<AssistantCmaAgentPage />);
    scroll.mockClear();
    mocks.history.mockReturnValue(
      history([
        turns[0]!,
        {
          ...turns[1]!,
          status: "completed",
          messages: [
            ...turns[1]!.messages,
            message(25, "assistant", "The page check is finished."),
          ],
        },
      ]),
    );
    rerender(<AssistantCmaAgentPage />);
    expect(screen.getByText("The page check is finished.")).toBeInTheDocument();
    expect(screen.getByText("Earlier setup reply")).toBeInTheDocument();
    expect(
      screen.getByRole("button", { name: "View new activity" }),
    ).toBeInTheDocument();
    expect(scroll).not.toHaveBeenCalled();
    fireEvent.click(screen.getByRole("button", { name: "View new activity" }));
    expect(scroll).toHaveBeenCalledTimes(1);
    expect(
      screen.queryByRole("button", { name: "View new activity" }),
    ).not.toBeInTheDocument();
  } finally {
    HTMLElement.prototype.scrollIntoView = previous;
  }
});

it("keeps older history in place without announcing it as new activity", () => {
  const scroll = vi.fn();
  const previous = HTMLElement.prototype.scrollIntoView;
  HTMLElement.prototype.scrollIntoView = scroll;
  try {
    mocks.history.mockReturnValue(history([turns[1]!]));
    const { rerender } = render(<AssistantCmaAgentPage />);
    scroll.mockClear();
    mocks.history.mockReturnValue(history());
    rerender(<AssistantCmaAgentPage />);
    expect(screen.getByText("Earlier setup reply")).toBeInTheDocument();
    expect(
      screen.queryByRole("button", { name: "View new activity" }),
    ).not.toBeInTheDocument();
    expect(scroll).not.toHaveBeenCalled();
  } finally {
    HTMLElement.prototype.scrollIntoView = previous;
  }
});

it("preserves an expanded long request across transcript refreshes", () => {
  const longRequest = "Keep all of this request visible when expanded. ".repeat(
    20,
  );
  mocks.history.mockReturnValue(
    history([
      {
        ...turns[0]!,
        messages: [message(9, "user", longRequest), turns[0]!.messages[1]!],
      },
      turns[1]!,
    ]),
  );
  const { rerender } = render(<AssistantCmaAgentPage />);
  fireEvent.click(screen.getByRole("button", { name: "Show full message" }));
  mocks.snapshot.mockReturnValue({ ...snapshot(), isFetchingNextPage: true });
  rerender(<AssistantCmaAgentPage />);
  expect(screen.getByRole("button", { name: "Show less" })).toHaveAttribute(
    "aria-expanded",
    "true",
  );
  expect(screen.getByText(longRequest.trim())).toBeInTheDocument();
});

it("scrolls to a verified historical request once and never to an unverified link", () => {
  const targets: HTMLElement[] = [];
  const previous = HTMLElement.prototype.scrollIntoView;
  HTMLElement.prototype.scrollIntoView = function () {
    targets.push(this);
  };
  try {
    linkedResponse();
    const { rerender, unmount } = render(<AssistantCmaAgentPage />);
    expect(targets).toHaveLength(1);
    expect(targets[0]).toBe(
      screen.getByRole("region", { name: "Linked request" }),
    );
    mocks.snapshot.mockReturnValue({ ...snapshot(), isFetchingNextPage: true });
    rerender(<AssistantCmaAgentPage />);
    expect(targets).toHaveLength(1);
    unmount();
    targets.length = 0;
    linkedResponse("agt-other");
    render(<AssistantCmaAgentPage />);
    expect(targets).toHaveLength(0);
  } finally {
    HTMLElement.prototype.scrollIntoView = previous;
  }
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
