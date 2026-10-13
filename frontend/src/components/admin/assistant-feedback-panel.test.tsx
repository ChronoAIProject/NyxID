import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { AssistantFeedbackPanel } from "./assistant-feedback-panel";
const { get } = vi.hoisted(() => ({ get: vi.fn() }));
vi.mock("@/lib/api-client", () => ({ api: { get } }));

function renderPanel() {
  render(
    <QueryClientProvider client={new QueryClient()}>
      <AssistantFeedbackPanel />
    </QueryClientProvider>,
  );
}

describe("AssistantFeedbackPanel", () => {
  beforeEach(() => {
    get.mockReset();
    get.mockResolvedValue({
      counts: [{ day: "2026-01-01", up: 2, down: 1 }],
      recent: [
        {
          message_id: "m",
          vote: "up",
          created_at: "2026-01-01T00:00:00Z",
          updated_at: "2026-01-01T00:00:00Z",
        },
      ],
    });
  });
  it("renders filters, timestamps and paging data", async () => {
    renderPanel();
    await waitFor(() => expect(screen.getByText("2026-01-01")).toBeVisible());
    expect(screen.getByLabelText("From")).toBeVisible();
    expect(screen.getByLabelText("To")).toBeVisible();
    expect(screen.getByLabelText("Vote")).toBeVisible();
    expect(screen.getByRole("time")).toHaveAttribute(
      "datetime",
      "2026-01-01T00:00:00Z",
    );
  });
  it("shows a visible load error", async () => {
    get.mockRejectedValueOnce(new Error("offline"));
    renderPanel();
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Unable to load",
    );
  });
  it("shows reply context beside the vote and a pre-A1 empty state", async () => {
    get.mockResolvedValue({
      counts: [],
      recent: [
        {
          message_id: "with-context",
          vote: "down",
          comment: "missed the file",
          created_at: "2026-01-02T00:00:00Z",
          updated_at: "2026-01-02T00:00:00Z",
          context: {
            agent_kind: "specialist",
            model: "nyxagent/chat",
            error_code: "idle_timeout",
            latency_ms: 1500,
            tool_calls: 3,
            tool_errors: 1,
            tool_labels: ["github__list_issues", "mail__send"],
          },
        },
        {
          message_id: "legacy",
          vote: "up",
          created_at: "2026-01-01T00:00:00Z",
          updated_at: "2026-01-01T00:00:00Z",
        },
      ],
    });
    renderPanel();
    expect(
      await screen.findByText(
        "Agent: specialist · Model: nyxagent/chat · Error: idle_timeout · Latency: 1.5 s · Tools: 3 (1 failed)",
      ),
    ).toBeVisible();
    expect(screen.getByText("github__list_issues, mail__send")).toBeVisible();
    expect(screen.getByText("Not helpful: missed the file")).toBeVisible();
    expect(screen.getByText("No context (pre-A1 row)")).toBeVisible();
  });
  it("sends the model, agent kind and failed-only filters", async () => {
    renderPanel();
    await screen.findByText("2026-01-01");
    fireEvent.change(screen.getByLabelText("Model"), { target: { value: "nyxagent/chat" } });
    fireEvent.change(screen.getByLabelText("Agent"), { target: { value: "nyxbot" } });
    fireEvent.click(screen.getByLabelText("Failed replies only"));
    await waitFor(() =>
      expect(get).toHaveBeenLastCalledWith(
        "/admin/assistant/feedback?model=nyxagent%2Fchat&agent_kind=nyxbot&has_error=true",
      ),
    );
  });
});
