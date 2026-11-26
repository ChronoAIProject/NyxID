import { cleanup, render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { chainVerificationResponseSchema } from "@/schemas/admin-chain-verification";
import { AdminIntegrityPage } from "./admin-integrity";

const state = vi.hoisted(() => ({ diagnostics: [] as Record<string, unknown>[] }));
vi.mock("@/hooks/use-admin-chain-verification", () => ({
  useChainVerification: () => ({
    data: chainVerificationResponseSchema.parse({
      chains: [],
      startup_diagnostics: state.diagnostics,
    }),
  }),
  useRunChainVerification: () => ({ mutate: vi.fn(), isPending: false }),
}));
vi.mock("@/stores/auth-store", () => ({
  useAuthStore: (select: (value: { user: null }) => unknown) => select({ user: null }),
}));

function diagnostic(code: string, severity?: "info" | "action") {
  return {
    code,
    summary: `${code} summary`,
    detail: "7 requests are holding funds.",
    remediation: severity === "action"
      ? "Check database health and server logs, then reload this page."
      : "No action is needed. This inventory is read-only.",
    detected_at: "2026-11-17T01:00:00Z",
    ...(severity ? { severity } : {}),
  };
}

beforeEach(() => { state.diagnostics = []; });
afterEach(cleanup);

it("keeps information quiet without an action card or badge", () => {
  state.diagnostics = [diagnostic("billing_hold_backlog", "info"), diagnostic("billing_stuck_requests", "info")];
  const { container } = render(<AdminIntegrityPage />);
  expect(screen.queryByText("Action required")).not.toBeInTheDocument();
  expect(container.querySelector(".border-destructive\\/50")).toBeNull();
  expect(screen.getByText("billing_hold_backlog summary")).toBeVisible();
  expect(screen.getByText("billing_stuck_requests summary")).toBeVisible();
  expect(screen.getAllByText("No action is needed. This inventory is read-only.")).toHaveLength(2);
  expect(screen.queryByText(/Check database health/)).not.toBeInTheDocument();
});

it("shows an action card for one actionable entry and leaves information muted", () => {
  state.diagnostics = [diagnostic("billing_hold_backlog", "info"), diagnostic("billing_stuck_requests", "action")];
  const { container } = render(<AdminIntegrityPage />);
  const card = container.querySelector(".border-destructive\\/50")!;
  expect(within(card as HTMLElement).getByText("Action required")).toBeVisible();
  expect(within(card as HTMLElement).getByText("billing_stuck_requests summary")).toBeVisible();
  expect(screen.getByText("billing_hold_backlog summary")).not.toHaveClass("text-destructive");
  expect(within(card as HTMLElement).getByText("Check database health and server logs, then reload this page.")).toBeVisible();
  expect(within(card as HTMLElement).queryByText("No action is needed. This inventory is read-only.")).not.toBeInTheDocument();
});

it.each(["info", "action"] as const)("keeps %s technical details collapsed until the operator opens them", async (severity) => {
  state.diagnostics = [{ ...diagnostic("billing_hold_backlog", severity), technical_details: "complete=true scanned_rows=7" }];
  render(<AdminIntegrityPage />);
  const control = screen.getByText("Details");
  const disclosure = control.closest("details")!;
  expect(disclosure).not.toHaveAttribute("open");
  expect(screen.getByText("complete=true scanned_rows=7")).not.toBeVisible();
  await userEvent.click(control);
  expect(disclosure).toHaveAttribute("open");
  expect(screen.getByText("complete=true scanned_rows=7")).toBeVisible();
});

it("preserves the old-server allowlist and action rendering when fields are absent", () => {
  state.diagnostics = [diagnostic("billing_usage_warming"), diagnostic("billing_usage_coverage"), diagnostic("billing_hold_backlog")];
  const { container } = render(<AdminIntegrityPage />);
  const card = container.querySelector(".border-destructive\\/50")!;
  expect(within(card as HTMLElement).getByText("billing_hold_backlog summary")).toBeVisible();
  expect(within(card as HTMLElement).queryByText("billing_usage_coverage summary")).not.toBeInTheDocument();
  expect(screen.getByText(/billing_usage_warming summary/)).toBeVisible();
  expect(screen.queryByText("Details")).not.toBeInTheDocument();
});

it("uses explicit severity even for a formerly informational code", () => {
  state.diagnostics = [diagnostic("billing_usage_coverage", "action")];
  render(<AdminIntegrityPage />);
  expect(screen.getByText("Action required")).toBeVisible();
});
