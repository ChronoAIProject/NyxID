import { render, screen } from "@testing-library/react";
import { afterEach, describe, expect, it, vi } from "vitest";
import { DecisionReceipt } from "./decision-receipt";
import type { NyxAgentAcknowledgement } from "@/schemas/assistant-nyxagent";
const row: NyxAgentAcknowledgement = {
  id: "11111111-1111-4111-8111-111111111111", kind: "action", status: "allowed",
  summary: "Attach weekly-review 1.0 to NyxBot", decider: "user", decided_by: "user", reason: null,
  service_slug: null, service_name: null, tool_name: "nyxid__set_agent_skills",
  confirmation: { risk: "sensitive", access_mode: "nyxmode", reason: "access_mode" },
  created_at: "2026-10-14T00:00:00Z", decided_at: "2026-10-14T00:01:00Z", expires_at: "2026-10-14T00:10:00Z",
};
afterEach(() => vi.useRealTimers());
describe("DecisionReceipt", () => {
  it.each([
    ["allowed", "user", "You allowed"], ["denied", "user", "You denied"],
    ["allowed", "orchestrator", "NyxBot allowed"], ["denied", "orchestrator", "NyxBot denied"],
    ["allowed", "future_decider", "Allowed"], ["denied", "future_decider", "Denied"],
    ["allowed", null, "Allowed"],
    ["used", "user", "Used"], ["expired", "user", "Expired"],
  ] as const)("renders %s by %s as a durable status", (status, decided_by, label) => {
    render(<DecisionReceipt acknowledgement={{ ...row, status, decided_by }} />);
    expect(screen.getByRole("status")).toHaveTextContent(`${label} · ${row.summary}`);
    expect(screen.getByRole("status")).toHaveTextContent("Sensitive");
    expect(screen.queryByRole("button")).not.toBeInTheDocument();
  });
  it.each([
    ["access_mode", ""], ["webhook", "bypassed automation confirmation"],
    ["policy", "bypassed machine / saved-login confirmation"], ["approval", "bypassed service approval"],
  ] as const)("explains the automatic %s bypass", (reason, phrase) => {
    render(<DecisionReceipt acknowledgement={{ ...row, decided_by: "access_mode", confirmation: { risk: "change", access_mode: "autopilot", reason } }} />);
    expect(screen.getByRole("status")).toHaveTextContent("Allowed automatically · Full access");
    if (phrase) expect(screen.getByRole("status")).toHaveTextContent(phrase);
    expect(screen.getByRole("status")).toHaveTextContent(row.summary);
  });
  it("uses the decided time and links an automatically published proposal", () => {
    vi.useFakeTimers(); vi.setSystemTime(new Date("2026-10-14T00:03:00Z"));
    render(<DecisionReceipt acknowledgement={{ ...row, decided_by: "access_mode", summary: "Skill weekly-review 1.0 published and attached to NyxBot", link: "https://nyxid.test/assistant?agent=a&proposal=p" }} />);
    expect(screen.getByRole("status")).toHaveTextContent("2m ago");
    expect(screen.getByText("· 2m ago")).toHaveAttribute("dateTime", row.decided_at);
    expect(screen.getByRole("link", { name: "Inspect or discard proposal" })).toHaveAttribute("href", "https://nyxid.test/assistant?agent=a&proposal=p");
  });
  it("explains client retirement without claiming expiry or success", () => {
    render(<DecisionReceipt acknowledgement={{ ...row, status: "pending", decided_by: null }} retired />);
    expect(screen.getByRole("status")).toHaveTextContent("No longer needed · Full access");
    expect(screen.getByRole("status")).not.toHaveTextContent("Expired");
    expect(screen.getByRole("status")).not.toHaveTextContent("You allowed");
  });
  it("leaves legacy receipt prose unchanged when no structured link exists", () => {
    render(<DecisionReceipt acknowledgement={{ ...row, summary: "Inspect or discard: https://nyxid.test/legacy" }} />);
    expect(screen.getByRole("status")).toHaveTextContent("Inspect or discard: https://nyxid.test/legacy");
    expect(screen.queryByRole("link")).not.toBeInTheDocument();
  });
});
