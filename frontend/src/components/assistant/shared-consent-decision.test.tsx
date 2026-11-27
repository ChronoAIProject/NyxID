import { act, cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { SharedConsentDecision } from "./shared-consent-decision";

const mocks = vi.hoisted(() => ({ decide: vi.fn(), saved: vi.fn() }));
vi.mock("@/hooks/use-shared-agents", () => ({ useDecideSharedConsent: () => ({ mutateAsync: mocks.decide, isPending: false, error: null }) }));
vi.mock("sonner", () => ({ toast: { success: mocks.saved } }));
afterEach(() => { cleanup(); vi.clearAllMocks(); });

it("keeps the continuation instruction visible after a decided card leaves the stack", async () => {
  let finish!: () => void;
  mocks.decide.mockReturnValue(new Promise<void>((resolve) => { finish = resolve; }));
  const view = render(<SharedConsentDecision agentId="agent" request={{
    id: "consent", connection_id: "connection", catalog_service_id: "service", operation: "POST /events",
    conversation_id: "chat", decision: "pending", expires_at: null, scope: "operation",
  }} />);
  fireEvent.click(screen.getByRole("button", { name: "Review full request" }));
  fireEvent.click(screen.getByRole("button", { name: "Allow once" }));
  expect(mocks.decide).toHaveBeenCalledWith({ consent: "consent", decision: "once" });
  view.unmount();
  await act(async () => { finish(); });
  expect(mocks.saved).toHaveBeenCalledWith("Choice saved. Continue the chat when ready.");
});
