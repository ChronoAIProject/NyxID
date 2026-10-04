import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { PoolEditor } from "./service-pools-tab";
import type { PoolCandidate, ServicePool } from "@/schemas/pools";

const mocks = vi.hoisted(() => ({
  update: vi.fn(),
  reload: vi.fn(),
  create: vi.fn(),
  candidates: vi.fn(),
  health: vi.fn(),
  reset: vi.fn(),
  pools: vi.fn(),
}));
vi.mock("@/hooks/use-pools", () => ({
  useUpdateServicePool: () => ({ mutateAsync: mocks.update, isPending: false }),
  useReloadServicePool: () => ({ mutateAsync: mocks.reload, isPending: false }),
  useCreateServicePool: () => ({ mutateAsync: mocks.create, isPending: false }),
  usePoolCandidates: mocks.candidates,
  useDeleteServicePool: () => ({ isPending: false }),
  usePoolHealth: mocks.health,
  useResetPoolHealth: () => ({ mutateAsync: mocks.reset, isPending: false }),
  useServicePools: mocks.pools,
}));
vi.mock("@/hooks/use-orgs", () => ({ useOrgs: mocks.pools }));
vi.mock("sonner", () => ({ toast: { success: vi.fn(), error: vi.fn() } }));

const candidate: PoolCandidate = {
  user_service_id: "member-id",
  name: "My connection",
  slug: "my-member",
  is_active: true,
  credential_binding: "user",
  protocol: "openai_completions",
  eligible: true,
  reason: null,
  catalog_service_id: "catalog-id",
  requires_compatibility_declaration: false,
  cooldown_until: null,
  consecutive_failures: 0,
  last_status: null,
};
const pool: ServicePool = {
  id: "pool-id",
  user_id: "owner",
  slug: "ai-route",
  name: "My pool",
  strategy: "priority",
  member_contract: "same_api",
  config_revision: 17,
  tier_balance: "round_robin",
  failover: null,
  members: [
    {
      user_service_id: "member-id",
      weight: 2,
      enabled: true,
      priority: 0,
      model: null,
      same_api_compatible: true,
    },
  ],
  rr_counter: 0,
  is_active: true,
  created_at: "2026-01-01",
  updated_at: "2026-01-01",
};
const page = (rows: PoolCandidate[]) => ({
  data: { pages: [{ candidates: rows, has_more: false, next_cursor: null }] },
  isLoading: false,
});
const saveButton = () => screen.getByRole("button", { name: "Save" });

beforeEach(() => {
  vi.clearAllMocks();
  mocks.update.mockResolvedValue(pool);
  mocks.create.mockResolvedValue(pool);
  mocks.pools.mockReturnValue({ data: [], isLoading: false });
  mocks.health.mockReturnValue({
    data: {
      candidates: [candidate],
      operation_checked: false,
      method: null,
      path: null,
    },
  });
  mocks.reset.mockResolvedValue({ reset: true });
  mocks.candidates.mockReturnValue(page([candidate]));
});

describe("pool routing controls", () => {
  it("shows truthful weighted shares and cycle order while editing", async () => {
    const user = userEvent.setup();
    const second = {
      ...candidate,
      user_service_id: "second-id",
      name: "Second connection",
      slug: "second",
    };
    mocks.candidates.mockReturnValue(page([candidate, second]));
    mocks.health.mockReturnValue({
      data: {
        candidates: [candidate, second],
        operation_checked: false,
        method: null,
        path: null,
      },
    });
    render(
      <PoolEditor
        pool={{
          ...pool,
          strategy: "weighted",
          members: [
            { ...pool.members[0]!, weight: 3 },
            { ...pool.members[0]!, user_service_id: "second-id", weight: 1 },
          ],
        }}
        onClose={vi.fn()}
      />,
    );
    expect(screen.getByText(/configured share 75%/)).toBeVisible();
    expect(screen.getByText(/configured share 25%/)).toBeVisible();
    expect(
      screen.getByText(/Cycle position 1 of 2 · first in cycle/),
    ).toBeVisible();
    expect(
      screen.getByText(/Cycle position 2 of 2 · last in cycle/),
    ).toBeVisible();
    expect(
      screen.getByText(
        /Cycle order: My connection \(3 turns\) → Second connection \(1 turn\)/,
      ),
    ).toBeVisible();
    await user.click(
      screen.getByRole("button", {
        name: "Move connection 2 earlier in the cycle",
      }),
    );
    expect(
      screen.getByText(
        /Cycle order: Second connection \(1 turn\) → My connection \(3 turns\)/,
      ),
    ).toBeVisible();
    await user.clear(screen.getByLabelText("Weight for member 1"));
    expect(
      screen.getAllByText(
        /configured share unavailable until enabled weights are valid/,
      ),
    ).toHaveLength(2);
    expect(screen.queryByText(/NaN%|Infinity%/)).not.toBeInTheDocument();
    await user.click(screen.getByRole("switch", { name: "Member 1 enabled" }));
    expect(
      screen.getByText(
        /Disabled · excluded from cycle · excluded from configured share/,
      ),
    ).toBeVisible();
    expect(screen.getByText(/configured share 100%/)).toBeVisible();
  });
  it("keeps weighted share displays neutral for invalid values and precise for tiny valid shares", async () => {
    const first = { ...pool.members[0]!, weight: 1 };
    const second = {
      ...pool.members[0]!,
      user_service_id: "second-id",
      weight: 1000,
    };
    const secondRow = {
      ...candidate,
      user_service_id: "second-id",
      name: "Second connection",
      slug: "second",
    };
    mocks.candidates.mockReturnValue(page([candidate, secondRow]));
    mocks.health.mockReturnValue({
      data: {
        candidates: [candidate, secondRow],
        operation_checked: false,
        method: null,
        path: null,
      },
    });
    const user = userEvent.setup();
    render(
      <PoolEditor
        pool={{ ...pool, strategy: "weighted", members: [first, second] }}
        onClose={vi.fn()}
      />,
    );
    expect(screen.getByText(/configured share <0.1%/)).toBeVisible();
    expect(screen.getByText(/configured share 99.9%/)).toBeVisible();

    for (const value of ["", "1.5", "1001", "0", "-1"]) {
      fireEvent.change(screen.getByLabelText("Weight for member 2"), {
        target: { value },
      });
      expect(
        screen.getAllByText(
          /configured share unavailable until enabled weights are valid/,
        ),
      ).toHaveLength(2);
      expect(
        screen.queryByText(/configured share [0-9]|NaN%|Infinity%/),
      ).not.toBeInTheDocument();
      await waitFor(() => expect(saveButton()).toBeDisabled());
    }
    fireEvent.change(screen.getByLabelText("Weight for member 2"), {
      target: { value: "3" },
    });
    expect(screen.getByText(/configured share 75%/)).toBeVisible();
    await user.click(
      screen.getByRole("button", {
        name: "Move connection 2 earlier in the cycle",
      }),
    );
    await user.click(saveButton());
    expect(mocks.update).toHaveBeenCalledWith(
      expect.objectContaining({
        expected_revision: 17,
        members: [
          expect.objectContaining({ user_service_id: "second-id", weight: 3 }),
          expect.objectContaining({ user_service_id: "member-id", weight: 1 }),
        ],
      }),
    );
  });
  it("invalidates only the affected weighted priority tier and reports invalid priorities neutrally", async () => {
    const first = { ...pool.members[0]!, weight: 3, priority: 0 };
    const second = {
      ...pool.members[0]!,
      user_service_id: "second-id",
      weight: 1,
      priority: 0,
    };
    const third = {
      ...pool.members[0]!,
      user_service_id: "third-id",
      weight: 1001,
      priority: 1,
    };
    const rows = [
      candidate,
      {
        ...candidate,
        user_service_id: "second-id",
        name: "Second connection",
        slug: "second",
      },
      {
        ...candidate,
        user_service_id: "third-id",
        name: "Third connection",
        slug: "third",
      },
    ];
    mocks.candidates.mockReturnValue(page(rows));
    mocks.health.mockReturnValue({
      data: {
        candidates: rows,
        operation_checked: false,
        method: null,
        path: null,
      },
    });
    const user = userEvent.setup();
    render(
      <PoolEditor
        pool={{
          ...pool,
          strategy: "priority",
          tier_balance: "weighted",
          members: [first, second, third],
        }}
        onClose={vi.fn()}
      />,
    );
    expect(screen.getByText(/configured tier share 75%/)).toBeVisible();
    expect(screen.getByText(/configured tier share 25%/)).toBeVisible();
    expect(
      screen.getByText(
        /configured share unavailable until enabled weights in this tier are valid/,
      ),
    ).toBeVisible();
    fireEvent.change(screen.getByLabelText("Weight for member 2"), {
      target: { value: "1.5" },
    });
    expect(
      screen.getAllByText(
        /configured share unavailable until enabled weights in this tier are valid/,
      ),
    ).toHaveLength(3);
    expect(screen.queryByText(/configured tier share/)).not.toBeInTheDocument();
    await user.click(screen.getByRole("switch", { name: "Member 2 enabled" }));
    expect(screen.getByText(/configured tier share 100%/)).toBeVisible();
    expect(
      screen.getByText(/disabled · excluded from tier share/),
    ).toBeVisible();
    await user.clear(screen.getByLabelText("Priority for member 3"));
    expect(
      screen.getByText(
        /Priority tier needs a valid number · configured share unavailable/,
      ),
    ).toBeVisible();
    expect(
      screen.queryByText(/Priority tier NaN|NaN%|Infinity%/),
    ).not.toBeInTheDocument();
    await waitFor(() => expect(saveButton()).toBeDisabled());
  });
});

it.each(["Round robin", "Automatic fallback"])(
  "can save %s after hiding an invalid weighted draft",
  async (routing) => {
    const user = userEvent.setup();
    render(
      <PoolEditor pool={{ ...pool, strategy: "weighted" }} onClose={vi.fn()} />,
    );
    await user.clear(screen.getByLabelText("Weight for member 1"));
    await waitFor(() => expect(saveButton()).toBeDisabled());
    await user.click(screen.getByRole("combobox", { name: "Routing" }));
    await user.click(screen.getByRole("option", { name: routing }));
    expect(
      screen.queryByLabelText("Weight for member 1"),
    ).not.toBeInTheDocument();
    await waitFor(() => expect(saveButton()).toBeEnabled());
    await user.click(saveButton());
    expect(mocks.update).toHaveBeenCalledWith(
      expect.objectContaining({
        strategy: routing === "Round robin" ? "round_robin" : "priority",
        members: [expect.objectContaining({ weight: 1 })],
      }),
    );
  },
);

it("can save Take turns after hiding an invalid tier weight", async () => {
  const user = userEvent.setup();
  render(
    <PoolEditor
      pool={{ ...pool, tier_balance: "weighted" }}
      onClose={vi.fn()}
    />,
  );
  await user.clear(screen.getByLabelText("Weight for member 1"));
  await user.click(
    screen.getByRole("combobox", {
      name: "Connections with the same priority",
    }),
  );
  await user.click(screen.getByRole("option", { name: "Take turns" }));
  await waitFor(() => expect(saveButton()).toBeEnabled());
  await user.click(saveButton());
  expect(mocks.update).toHaveBeenCalledWith(
    expect.objectContaining({
      tier_balance: "round_robin",
      members: [expect.objectContaining({ weight: 1 })],
    }),
  );
});

it("retains priorities, models, tier balancing and retries across a routing round trip", async () => {
  const { defaultFailoverPolicy } = await import("@/schemas/pools");
  const user = userEvent.setup();
  const original = {
    ...pool,
    member_contract: "ai_chat" as const,
    tier_balance: "weighted" as const,
    failover: { ...defaultFailoverPolicy, max_attempts: 4 },
    members: [{ ...pool.members[0]!, priority: 10, model: "chat-model" }],
  };
  render(<PoolEditor pool={original} onClose={vi.fn()} />);
  for (const routing of ["Round robin", "Automatic fallback"]) {
    await user.click(screen.getByRole("combobox", { name: "Routing" }));
    await user.click(screen.getByRole("option", { name: routing }));
  }
  expect(screen.getByLabelText("Priority for member 1")).toHaveValue(10);
  expect(screen.getByLabelText("Model for member 1")).toHaveValue("chat-model");
  expect(
    screen.getByRole("combobox", {
      name: "Connections with the same priority",
    }),
  ).toHaveTextContent("Share by weight");
  await user.type(screen.getByLabelText("Name"), " changed");
  await user.click(saveButton());
  expect(mocks.update).toHaveBeenCalledWith(
    expect.objectContaining({
      members: original.members,
      failover: original.failover,
      tier_balance: "weighted",
      member_contract: "ai_chat",
    }),
  );
});

it.each(["weighted", "round_robin"] as const)(
  "moves connections within their %s priority tier without changing settings",
  async (tierBalance) => {
    const user = userEvent.setup();
    const first = { ...pool.members[0]!, weight: 2, priority: 0 };
    const backup = { ...first, user_service_id: "backup", priority: 10 };
    const second = { ...first, user_service_id: "second", weight: 1 };
    render(
      <PoolEditor
        pool={{
          ...pool,
          tier_balance: tierBalance,
          members: [first, backup, second],
        }}
        onClose={vi.fn()}
      />,
    );
    expect(
      screen.getByRole("button", {
        name: "Move connection 1 earlier within priority 0",
      }),
    ).toBeDisabled();
    await user.click(
      screen.getByRole("button", {
        name: "Move connection 2 earlier within priority 0",
      }),
    );
    await user.click(saveButton());
    expect(mocks.update).toHaveBeenCalledWith(
      expect.objectContaining({ members: [second, backup, first] }),
    );
  },
);

it("reloads a conflicted draft and saves using the latest revision", async () => {
  const { ApiError } = await import("@/lib/api-client");
  const user = userEvent.setup();
  mocks.update.mockRejectedValueOnce(
    new ApiError(409, {
      error: "conflict",
      error_code: 1009,
      message: "Changed elsewhere",
    }),
  );
  mocks.reload.mockResolvedValue({
    ...pool,
    name: "Latest pool",
    config_revision: 18,
  });
  render(<PoolEditor pool={pool} onClose={vi.fn()} />);
  await user.type(screen.getByLabelText("Name"), " stale");
  await user.click(saveButton());
  await user.click(
    await screen.findByRole("button", { name: "Reload latest" }),
  );
  await waitFor(() =>
    expect(screen.getByLabelText("Name")).toHaveValue("Latest pool"),
  );
  await user.type(screen.getByLabelText("Name"), " edited");
  await user.click(saveButton());
  expect(mocks.update).toHaveBeenLastCalledWith(
    expect.objectContaining({
      name: "Latest pool edited",
      expected_revision: 18,
    }),
  );
});
