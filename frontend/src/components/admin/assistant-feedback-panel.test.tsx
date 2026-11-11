import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { AssistantFeedbackPanel } from "./assistant-feedback-panel";
const { get } = vi.hoisted(() => ({ get: vi.fn() }));
vi.mock("@/lib/api-client", async (original) => ({
  ...(await original<typeof import("@/lib/api-client")>()),
  api: { get },
}));
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

it("requests a thread only on demand, prepends previous messages, and anchors the rated reply", async () => {
  get.mockImplementation((path: string) =>
    Promise.resolve(
      path.includes("/thread")
        ? {
            unavailable: false,
            messages: path.includes("cursor=")
              ? [
                  {
                    id: "previous",
                    role: "user",
                    text: "Earlier question",
                    created_at: row.created_at,
                    rated: false,
                    truncated: false,
                  },
                ]
              : [
                  {
                    id: "rated",
                    role: "assistant",
                    text: "Rated answer",
                    created_at: row.created_at,
                    rated: true,
                    truncated: false,
                  },
                ],
            next_cursor: path.includes("cursor=") ? null : "bound-cursor",
          }
        : {
            counts: [],
            recent: [{ ...row, id: "feedback-1" }],
            can_preview_threads: true,
          },
    ),
  );
  renderPanel();
  await screen.findByText("Preview message thread");
  expect(
    get.mock.calls.some(([path]) => String(path).includes("/thread")),
  ).toBe(false);
  await userEvent.click(
    screen.getByRole("button", { name: "Preview message thread" }),
  );
  expect(await screen.findByText("Rated answer")).toBeVisible();
  expect(screen.getByText("Assistant · Rated reply")).toBeVisible();
  await userEvent.click(
    screen.getByRole("button", { name: "Load previous messages" }),
  );
  expect(await screen.findByText("Earlier question")).toBeVisible();
  const messages = screen
    .getByRole("region", { name: "Feedback message thread" })
    .querySelectorAll("li");
  expect(messages[0]).toHaveTextContent("Earlier question");
  expect(messages[1]).toHaveTextContent("Rated answer");
});
it("keeps two ratings of the same group reply independent and hides previews from metadata-only readers", async () => {
  get.mockResolvedValue({
    counts: [],
    recent: [
      { ...row, id: "rater-1", comment: "Good" },
      { ...row, id: "rater-2", comment: "Needs work" },
    ],
    can_preview_threads: false,
  });
  renderPanel();
  expect(await screen.findByText("Good")).toBeVisible();
  expect(screen.getByText("Needs work")).toBeVisible();
  expect(
    screen.queryByRole("button", { name: "Preview message thread" }),
  ).not.toBeInTheDocument();
});
it("clears matching totals and context when metadata authorization is revoked", async () => {
  const { ApiError } = await import("@/lib/api-client");
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false } },
  });
  get.mockResolvedValue({
    counts: [{ day: "2026-01-01", up: 2, down: 1 }],
    recent: [{ ...row, id: "rating" }],
    can_preview_threads: true,
  });
  render(
    <QueryClientProvider client={client}>
      <AssistantFeedbackPanel />
    </QueryClientProvider>,
  );
  await screen.findByLabelText("Matching feedback totals");
  get.mockRejectedValue(
    new ApiError(403, {
      error: "forbidden",
      error_code: 1003,
      message: "Forbidden",
    }),
  );
  await client.invalidateQueries();
  await screen.findByRole("alert");
  expect(
    screen.queryByLabelText("Matching feedback totals"),
  ).not.toBeInTheDocument();
  expect(
    screen.queryByRole("button", { name: "Preview message thread" }),
  ).not.toBeInTheDocument();
});
