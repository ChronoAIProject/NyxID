import { fireEvent, render, screen, within } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { NyxbotUsage } from "./nyxbot-usage";
import { assistantAnalyticsSchema } from "@/schemas/assistant-analytics";

const mocks = vi.hoisted(() => ({ analytics: vi.fn(), refetch: vi.fn() }));
vi.mock("@/hooks/use-assistant-analytics", () => ({ useAssistantAnalytics: mocks.analytics }));
vi.mock("@/hooks/use-assistant-feedback-admin", () => ({
  useAssistantFeedbackAdmin: () => ({ data: { counts: [], recent: [] } }),
}));
vi.mock("./assistant-feedback-panel", () => ({ AssistantFeedbackPanel: () => <div>Reply feedback</div> }));
vi.mock("./nyxbot-model-usage", () => ({ NyxbotModelUsage: () => <div>Model usage</div> }));
vi.mock("./nyxbot-real-work", () => ({ NyxbotRealWork: () => <div>Weekly results</div> }));
vi.mock("recharts", async (original) => ({
  ...(await original<typeof import("recharts")>()),
  ResponsiveContainer: () => null,
}));

it("keeps activity, outcomes and other sections visible when only real work is unavailable", () => {
  const data = assistantAnalyticsSchema.parse({
    from: "2026-11-01T00:00:00Z", to: "2026-11-08T00:00:00Z", generated_at: "2026-11-08T00:00:00Z",
    active_sessions: 17, active_accounts: 4, running_now: 2,
    outcomes: { completed: 11, failed: 3, stopped: 1, with_tools: 9, with_tool_errors: 1 },
    daily_sessions: [], daily_turns: [], real_work: null, real_work_unavailable: true,
  });
  mocks.analytics.mockReturnValue({ data, refetch: mocks.refetch, isError: false, isPending: false });
  render(<NyxbotUsage />);
  expect(screen.getByText("Weekly real work is unavailable.")).toBeInTheDocument();
  expect(within(screen.getByText("Active sessions", { selector: "p" }).parentElement!).getByText("17")).toBeInTheDocument();
  expect(screen.getByText("Conversations with user input · 4 accounts")).toBeInTheDocument();
  expect(screen.getByText("11")).toBeInTheDocument();
  expect(screen.getByText("Model usage")).toBeInTheDocument();
  expect(screen.getByText("Reply feedback")).toBeInTheDocument();
  expect(screen.queryByText("Unable to load NyxBot activity.")).not.toBeInTheDocument();
  expect(screen.queryByText("Weekly results")).not.toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Retry" }));
  expect(mocks.refetch).toHaveBeenCalledOnce();
});
