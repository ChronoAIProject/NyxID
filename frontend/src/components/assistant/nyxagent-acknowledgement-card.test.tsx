import { cleanup, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { NyxAgentAcknowledgementCard } from "./nyxagent-acknowledgement-card";
import type { NyxAgentAcknowledgement } from "@/schemas/assistant-nyxagent";

const acknowledgement: NyxAgentAcknowledgement = {
  id: "12345678-1234-4123-8123-123456789012",
  kind: "service",
  status: "pending",
  summary: "Delete agent key ci-bot",
  decider: "user",
  decided_by: null,
  reason: null,
  service_slug: "github",
  service_name: "GitHub",
  tool_name: null,
  created_at: "2026-09-17T00:00:00Z",
  decided_at: null,
  expires_at: "2026-09-17T00:15:00Z",
};
afterEach(cleanup);

it("hides a stale owner ask in Full access while keeping specialist requests routed to NyxBot", () => {
  const decide = vi.fn();
  const view = render(<NyxAgentAcknowledgementCard acknowledgement={acknowledgement} accessMode="autopilot" deciding={false} onDecision={decide} />);
  expect(screen.getByRole("status")).toHaveTextContent("No longer needed · Full access · Delete agent key ci-bot");
  expect(screen.queryByRole("button")).not.toBeInTheDocument();
  expect(decide).not.toHaveBeenCalled();
  view.rerender(<NyxAgentAcknowledgementCard acknowledgement={{ ...acknowledgement, decider: "orchestrator" }} accessMode="autopilot" deciding={false} onDecision={decide} />);
  expect(screen.getByText("Requested from NyxBot")).toBeVisible();
  expect(screen.getByRole("button", { name: "Allow" })).toBeVisible();
});

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

it.each([
  ["service", "Allow this agent to use GitHub?"],
  ["account", "Allow this agent to read your NyxID account?"],
] as const)("labels a specialist's %s request as routed to NyxBot", async (kind, text) => {
  const decide = vi.fn().mockResolvedValue(undefined);
  render(
    <NyxAgentAcknowledgementCard
      acknowledgement={{ ...acknowledgement, kind, decider: "orchestrator" }}
      deciding={false}
      onDecision={decide}
    />,
  );
  const card = screen.getByRole("region", { name: text });
  expect(card).toHaveTextContent("Requested from NyxBot");
  // The user can still decide it from the specialist's thread.
  fireEvent.click(screen.getByRole("button", { name: "Allow" }));
  await waitFor(() => expect(decide).toHaveBeenCalledWith("allow"));
});

it.each([
  ["orchestrator", "allowed", "Scope matches the user's request", "NyxBot allowed"],
  ["orchestrator", "denied", null, "NyxBot denied"],
  ["user", "denied", null, "You denied"],
  ["user", "allowed", null, "You allowed"],
] as const)(
  "shows who decided a routed request (%s, %s)",
  (decidedBy, status, reason, text) => {
    render(
      <NyxAgentAcknowledgementCard
        acknowledgement={{
          ...acknowledgement,
          decider: "orchestrator",
          decided_by: decidedBy,
          status,
          reason,
        }}
        deciding={false}
        onDecision={vi.fn()}
      />,
    );
    expect(screen.getByRole("status")).toHaveTextContent(
      `${text} · ${acknowledgement.summary}`,
    );
  },
);

it.each([
  [{ risk: "destructive", access_mode: "nyxmode", reason: "access_mode" }, "Destructive", "Your access mode is Auto."],
  [{ risk: "change", access_mode: "ask_every_time", reason: "access_mode" }, "Change", "Your access mode is Ask every time."],
  [{ risk: "sensitive", access_mode: null, reason: "webhook" }, "Sensitive", "This automation's confirmation policy asks."],
  [{ risk: "change", reason: "policy" }, "Change", "This machine's or saved login's confirmation setting asks."],
] as const)("explains an action card's risk and why it asks (%o)", (confirmation, risk, why) => {
  render(
    <NyxAgentAcknowledgementCard
      acknowledgement={{ ...acknowledgement, kind: "action", confirmation }}
      deciding={false}
      onDecision={vi.fn()}
    />,
  );
  expect(screen.getByText(risk)).toBeVisible();
  expect(screen.getByText(why, { exact: false })).toBeVisible();
  expect(screen.getByText("Confirm: Delete agent key ci-bot")).toBeVisible();
});

it("shows no risk line on cards from older servers", () => {
  render(
    <NyxAgentAcknowledgementCard
      acknowledgement={{ ...acknowledgement, kind: "action" }}
      deciding={false}
      onDecision={vi.fn()}
    />,
  );
  expect(screen.queryByText(/access mode is/)).not.toBeInTheDocument();
});

it.each(["skills", "operations"] as const)("explains that routed %s widening follows the permission mode", (kind) => {
  render(<NyxAgentAcknowledgementCard acknowledgement={{ ...acknowledgement, kind, decider: "orchestrator" }} deciding={false} onDecision={vi.fn()} />);
  expect(screen.getByRole("region")).toHaveTextContent("follows NyxBot's permission mode");
  expect(screen.getByRole("region")).not.toHaveTextContent("Widening requires your confirmation");
  expect(screen.getByRole("region")).not.toHaveTextContent("You will get a separate card");
});
