import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { NyxAgentAcknowledgementCard } from "./nyxagent-acknowledgement-card";
import type { NyxAgentAcknowledgement } from "@/schemas/assistant-nyxagent";

const acknowledgement: NyxAgentAcknowledgement = {
  id: "12345678-1234-4123-8123-123456789012",
  kind: "service",
  status: "pending",
  summary: "Delete agent key ci-bot",
  service_slug: "github",
  service_name: "GitHub",
  tool_name: null,
  created_at: "2026-09-17T00:00:00Z",
  decided_at: null,
  expires_at: "2026-09-17T00:15:00Z",
};
afterEach(cleanup);

it.each([
  ["service", "Allow this chat to use GitHub?"],
  ["account", "Allow this chat to manage your NyxID account"],
  ["action", "Confirm: Delete agent key ci-bot"],
] as const)("shows %s copy and waits for an explicit human decision", async (kind, text) => {
  const decide = vi.fn().mockResolvedValue(undefined);
  render(
    <NyxAgentAcknowledgementCard
      acknowledgement={{ ...acknowledgement, kind }}
      deciding={false}
      onDecision={decide}
    />,
  );
  expect(screen.getByText(text, { exact: false })).toBeVisible();
  expect(decide).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "Allow" }));
  await waitFor(() => expect(decide).toHaveBeenCalledWith("allow"));
});

it("supports Deny, blocks while saving, and shows errors without deciding automatically", async () => {
  const decide = vi.fn().mockRejectedValue(new Error("Try again later"));
  const view = render(
    <NyxAgentAcknowledgementCard acknowledgement={acknowledgement} deciding onDecision={decide} />,
  );
  expect(screen.getByRole("button", { name: "Deny" })).toBeDisabled();
  view.rerender(
    <NyxAgentAcknowledgementCard
      acknowledgement={acknowledgement}
      deciding={false}
      onDecision={decide}
    />,
  );
  fireEvent.click(screen.getByRole("button", { name: "Deny" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("Try again later");
  expect(decide).toHaveBeenCalledExactlyOnceWith("deny");
});

it.each(["allowed", "denied", "expired", "used"] as const)(
  "renders %s as a compact status without controls",
  (status) => {
    render(
      <NyxAgentAcknowledgementCard
        acknowledgement={{ ...acknowledgement, status }}
        deciding={false}
        onDecision={vi.fn()}
      />,
    );
    expect(screen.getByRole("status")).toHaveTextContent(new RegExp(status, "i"));
    expect(screen.queryByRole("button")).not.toBeInTheDocument();
  },
);
