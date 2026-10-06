import { render, screen, within } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { ServicePoolRoutingPanel } from "./service-pool-routing-panel";
import type { PoolCandidate, ServicePool } from "@/schemas/pools";
const state = vi.hoisted(() => ({
  health: vi.fn(),
  result: {
    data: { candidates: [] as PoolCandidate[] },
    isError: false,
    isLoading: false,
  },
}));
vi.mock("@/hooks/use-pools", () => ({
  usePoolHealth: (options: unknown) => {
    state.health(options);
    return state.result;
  },
}));
const pool: ServicePool = {
  id: "pool",
  user_id: "me",
  name: "Twitter route",
  slug: "twitter-route",
  strategy: "priority",
  member_contract: "same_api",
  config_revision: 4,
  tier_balance: "round_robin",
  failover: null,
  members: [
    { user_service_id: "backup", enabled: true, priority: 10, weight: 1 },
    { user_service_id: "platform", enabled: true, priority: 0, weight: 1 },
  ],
  rr_counter: 0,
  is_active: true,
  created_at: "2026-01-01",
  updated_at: "2026-01-01",
};
function view(overrides: Partial<ServicePool> = {}) {
  return (
    <ServicePoolRoutingPanel
      pool={{ ...pool, ...overrides }}
      connections={[]}
      insights={{
        connections: new Map(),
        status: "unavailable",
        refresh: vi.fn(),
      }}
    />
  );
}
beforeEach(() => {
  state.result = { data: { candidates: [] }, isError: false, isLoading: false };
  state.health.mockClear();
});
describe("inline saved pool route", () => {
  it("shows actual priorities and unknown health without inventing free billing or readiness", () => {
    render(view());
    const rows = within(screen.getByRole("table")).getAllByRole("row");
    expect(within(rows[1]!).getByText("Priority 0")).toBeVisible();
    expect(within(rows[2]!).getByText("Priority 10")).toBeVisible();
    expect(screen.getAllByText("Not inspected")).toHaveLength(2);
    expect(screen.queryByText("No NyxID charge")).not.toBeInTheDocument();
    expect(
      screen.getByText(/Platform-key usage bills the acting person/),
    ).toBeVisible();
  });
  it("inspects the submitted operation without issuing service calls while editing the path", async () => {
    const user = userEvent.setup();
    render(view());
    await user.selectOptions(
      screen.getByLabelText("Method for Twitter route"),
      "GET",
    );
    await user.clear(screen.getByLabelText("Operation path for Twitter route"));
    await user.type(
      screen.getByLabelText("Operation path for Twitter route"),
      "/2/users/me",
    );
    expect(state.health).toHaveBeenLastCalledWith(
      expect.objectContaining({ method: "POST", path: "/" }),
    );
    await user.click(screen.getByRole("button", { name: "Inspect" }));
    expect(state.health).toHaveBeenLastCalledWith(
      expect.objectContaining({ method: "GET", path: "/2/users/me" }),
    );
  });
  it("does not retain eligibility after an inspection error", () => {
    state.result = {
      data: {
        candidates: [
          {
            user_service_id: "platform",
            eligible: true,
            reason: null,
            consecutive_failures: 0,
          } as PoolCandidate,
        ],
      },
      isError: true,
      isLoading: false,
    };
    render(view());
    expect(
      screen.queryByText("Eligible", { exact: true }),
    ).not.toBeInTheDocument();
    expect(screen.getByText(/connection health is unverified/)).toBeVisible();
  });
  it("shows AI model mapping and the gateway alias", () => {
    render(
      view({
        member_contract: "ai_chat",
        members: [{ ...pool.members[0]!, model: "model-b" }],
      }),
    );
    expect(screen.getByText("pool:twitter-route")).toBeVisible();
    expect(screen.getByText("Model: model-b")).toBeVisible();
    expect(state.health).toHaveBeenLastCalledWith(
      expect.objectContaining({ method: "POST", path: "chat/completions" }),
    );
  });
});
