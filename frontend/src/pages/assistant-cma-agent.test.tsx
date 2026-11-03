import { fireEvent, render, screen } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import { AssistantCmaAgentPage } from "./assistant-cma-agent";
const mocks = vi.hoisted(() => ({
  response: undefined as string | undefined,
  enabled: true,
  agent: vi.fn(),
  task: vi.fn(),
  history: vi.fn(),
  stream: vi.fn(),
}));
vi.mock("@tanstack/react-router", () => ({
  useParams: () => ({ agentId: "local-agent" }),
  useSearch: () => ({ response: mocks.response }),
  Link: ({ to, children }: { to: string; children: React.ReactNode }) => (
    <a href={to}>{children}</a>
  ),
}));
vi.mock("@/hooks/use-feature-flag", () => ({
  useFeature: () => mocks.enabled,
}));
vi.mock("@/hooks/use-nyxbot-cma-agents", () => ({
  useCmaAgent: mocks.agent,
  useCmaResponse: mocks.task,
  useCmaTranscript: mocks.history,
  useCmaEventSnapshot: () => ({ data: undefined }),
  useCmaEventStream: mocks.stream,
}));
vi.mock("@/components/assistant/cma-desktop-panel", () => ({
  CmaDesktopPanel: ({ agentId }: { agentId: string }) => (
    <div>Desktop for {agentId}</div>
  ),
}));
beforeEach(() => {
  vi.clearAllMocks();
  mocks.response = undefined;
  mocks.enabled = true;
  mocks.agent.mockReturnValue({
    data: {
      id: "local-agent",
      cma_agent_id: "agt-one",
      label: "Review Agent",
      status: "working",
      sandbox_id: "local-sandbox",
      cma_sandbox_id: "sbx-one",
      last_observed: null,
      available: true,
      unavailable_reason: null,
    },
    refetch: vi.fn(),
  });
  mocks.task.mockReturnValue({ data: undefined, refetch: vi.fn() });
  mocks.history.mockReturnValue({
    data: { pages: [{ messages: [], next_cursor: null }] },
    refetch: vi.fn(),
  });
  mocks.stream.mockReturnValue({
    events: [],
    state: "idle",
    reconnect: vi.fn(),
  });
});
it("never substitutes the latest task when the card has no response binding", () => {
  render(<AssistantCmaAgentPage />);
  expect(
    screen.getByText(/task binding is not available/i),
  ).toBeInTheDocument();
  expect(mocks.task).toHaveBeenCalledWith("local-agent", undefined, true);
});
it("labels the Agent’s full history as shared across chats", () => {
  render(<AssistantCmaAgentPage />);
  fireEvent.click(screen.getByRole("button", { name: "Agent history" }));
  expect(
    screen.getByText(/history is shared across NyxBot chats/i),
  ).toBeInTheDocument();
  expect(
    screen.queryByRole("button", {
      name: /stop agent|delete agent|create agent/i,
    }),
  ).not.toBeInTheDocument();
});
it("renders only the bound response and keeps its status explicit", () => {
  mocks.response = "rsp-one";
  mocks.task.mockReturnValue({
    data: {
      id: "rsp-one",
      agent_id: "agt-one",
      status: "in_progress",
      output: [
        {
          id: "message",
          type: "message",
          role: "assistant",
          content: [{ type: "output_text", text: "Observed result" }],
        },
      ],
    },
    refetch: vi.fn(),
  });
  render(<AssistantCmaAgentPage />);
  expect(screen.getByText("Observed result")).toBeInTheDocument();
  expect(screen.getByText("rsp-one")).toBeInTheDocument();
});
it("hides execution and desktop on direct flag-off access", () => {
  mocks.enabled = false;
  render(<AssistantCmaAgentPage />);
  expect(screen.getByText(/not enabled for this account/i)).toBeInTheDocument();
  expect(screen.queryByText("Review Agent")).not.toBeInTheDocument();
});
