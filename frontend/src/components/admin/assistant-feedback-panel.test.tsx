import { render, screen, waitFor } from "@testing-library/react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { AssistantFeedbackPanel } from "./assistant-feedback-panel";
const { get } = vi.hoisted(() => ({ get: vi.fn() }));
vi.mock("@/lib/api-client", () => ({ api: { get } }));
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
    render(<AssistantFeedbackPanel />);
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
    render(<AssistantFeedbackPanel />);
    expect(await screen.findByRole("alert")).toHaveTextContent(
      "Unable to load",
    );
  });
});
