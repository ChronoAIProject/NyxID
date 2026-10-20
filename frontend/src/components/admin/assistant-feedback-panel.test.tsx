import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { AssistantFeedbackPanel } from "./assistant-feedback-panel";
const { get } = vi.hoisted(() => ({ get: vi.fn() }));
vi.mock("@/lib/api-client", () => ({ api: { get } }));
const row = {
  message_id: "reply-id",
  vote: "up",
  created_at: "2026-01-01T00:00:00Z",
  updated_at: "2026-01-01T00:00:00Z",
};
function renderPanel(range?: URLSearchParams) {
  return render(
    <QueryClientProvider client={new QueryClient()}>
      <AssistantFeedbackPanel range={range} />
    </QueryClientProvider>,
  );
}
describe("AssistantFeedbackPanel", () => {
  beforeEach(() => {
    get.mockReset();
    get.mockResolvedValue({
      counts: [{ day: "2026-01-01", up: 2, down: 1 }],
      recent: [row],
    });
  });
  it("leads with the vote and comment and keeps the message id in collapsed details", async () => {
    renderPanel();
    expect(await screen.findByText("No comment provided")).toBeVisible();
    expect(screen.getByLabelText("Vote")).toBeVisible();
    expect(screen.getByLabelText("Agent")).toBeVisible();
    expect(screen.getByRole("time")).toHaveAttribute(
      "datetime",
      row.created_at,
    );
    expect(screen.getByText("reply-id")).not.toBeVisible();
    await userEvent.click(screen.getByText("Reply details"));
    expect(screen.getByText("reply-id")).toBeVisible();
    expect(screen.getByText("Reply details unavailable")).toBeVisible();
  });
  it("shows a retryable load error", async () => {
    get.mockRejectedValueOnce(new Error("offline"));
    renderPanel();
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Unable to load",
    );
    await userEvent.click(screen.getByRole("button", { name: "Retry" }));
    expect(await screen.findByText("No comment provided")).toBeVisible();
  });
  it("shows metadata without claiming failed replies can be rated", async () => {
    get.mockResolvedValue({
      counts: [],
      recent: [
        {
          ...row,
          vote: "down",
          comment: "missed the file",
          context: {
            agent_kind: "specialist",
            model: "nyxagent/chat",
            latency_ms: 1500,
            tool_calls: 3,
            tool_errors: 1,
            tool_labels: ["github__list_issues"],
          },
        },
      ],
    });
    renderPanel();
    expect(await screen.findByText("missed the file")).toBeVisible();
    expect(screen.getByText("1.5 s reply time")).toBeVisible();
    expect(screen.getByText("3 recorded tools · 1 error")).toBeVisible();
    expect(
      screen.queryByLabelText("Failed replies only"),
    ).not.toBeInTheDocument();
    await userEvent.click(screen.getByText("Reply details"));
    expect(screen.getByText("github__list_issues")).toBeVisible();
  });
  it("preserves the period while paging and resets the cursor after filtering", async () => {
    get.mockResolvedValueOnce({
      counts: [],
      recent: [row],
      next_cursor: "cursor-a",
    });
    renderPanel(new URLSearchParams({ from: "start", to: "end" }));
    await userEvent.click(
      await screen.findByRole("button", { name: "Next page" }),
    );
    await waitFor(() =>
      expect(get).toHaveBeenLastCalledWith(
        "/admin/assistant/feedback?from=start&to=end&cursor=cursor-a",
      ),
    );
    await userEvent.type(screen.getByLabelText("Model"), "nyxagent/chat");
    await userEvent.click(screen.getByRole("button", { name: "Apply" }));
    await waitFor(() =>
      expect(get).toHaveBeenLastCalledWith(
        "/admin/assistant/feedback?from=start&to=end&model=nyxagent%2Fchat",
      ),
    );
    await userEvent.click(screen.getByLabelText("Vote"));
    await userEvent.click(screen.getByRole("option", { name: "Not helpful" }));
    await waitFor(() =>
      expect(get).toHaveBeenLastCalledWith(
        "/admin/assistant/feedback?from=start&to=end&vote=down&model=nyxagent%2Fchat",
      ),
    );
  });
  it("shows a useful filtered empty state", async () => {
    get.mockResolvedValue({ counts: [], recent: [] });
    renderPanel();
    expect(
      await screen.findByText("No feedback in this period."),
    ).toBeVisible();
    await userEvent.click(screen.getByLabelText("Agent"));
    await userEvent.click(screen.getByRole("option", { name: "NyxBot" }));
    expect(
      await screen.findByText("No feedback matches these filters."),
    ).toBeVisible();
    expect(get).toHaveBeenLastCalledWith(
      "/admin/assistant/feedback?agent_kind=nyxbot",
    );
  });
});
