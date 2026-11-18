import { render, screen } from "@testing-library/react";
import { expect, it } from "vitest";
import { NyxAgentRequestLinks } from "./nyxagent-request-links";
import type { AssistantAgentRequest } from "@/schemas/assistant-nyxagent";

it("2072 links a routed request to its source conversation instead of implying a local card", () => {
  const request: AssistantAgentRequest = {
    request_id: "card", conversation_id: "source", agent: "Bookkeeper", agent_id: null,
    kind: "action", service_slug: "google-sheets", summary: "Write ledger", requested_by: null,
    expires_at: "2099-01-01", status: "pending", review_path: "/assistant?c=source&card=card",
  };
  const view = render(<NyxAgentRequestLinks requests={[request]} conversationId="receiver" />);
  expect(screen.getByText("Specialist requests in other conversations (1)")).toBeInTheDocument();
  expect(screen.getByRole("link", { hidden: true })).toHaveAttribute("href", request.review_path);
  view.rerender(<NyxAgentRequestLinks requests={[request]} conversationId="source" />);
  expect(screen.queryByRole("link", { hidden: true })).toBeNull();
  view.rerender(<NyxAgentRequestLinks requests={[{ ...request, review_path: undefined }]} conversationId="receiver" />);
  expect(screen.queryByText(/Specialist requests/)).toBeNull();
});
