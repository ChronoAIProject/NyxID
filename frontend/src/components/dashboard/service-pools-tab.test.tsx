import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  PoolEditor,
  PoolHealthDialog,
  ServicePoolsTab,
} from "./service-pools-tab";
import type { ServicePool } from "@/schemas/pools";
const mocks = vi.hoisted(() => ({
  update: vi.fn(),
  create: vi.fn(),
  candidates: vi.fn(),
  health: vi.fn(),
  reset: vi.fn(),
  pools: vi.fn(),
}));
vi.mock("@/hooks/use-pools", () => ({
  useUpdateServicePool: () => ({ mutateAsync: mocks.update, isPending: false }),
  useCreateServicePool: () => ({ mutateAsync: mocks.create, isPending: false }),
  usePoolCandidates: mocks.candidates,
  useDeleteServicePool: () => ({ isPending: false }),
  usePoolHealth: mocks.health,
  useResetPoolHealth: () => ({ mutateAsync: mocks.reset, isPending: false }),
  useServicePools: mocks.pools,
}));
vi.mock("@/hooks/use-orgs", () => ({
  useOrgs: () => ({
    data: [{ id: "org-id", display_name: "Research", your_role: "admin" }],
  }),
}));
vi.mock("sonner", () => ({ toast: { success: vi.fn(), error: vi.fn() } }));
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
beforeEach(() => {
  vi.clearAllMocks();
  mocks.update.mockResolvedValue(pool);
  mocks.pools.mockReturnValue({ data: [], isLoading: false });
  mocks.health.mockReturnValue({ data: { candidates: [] } });
  mocks.reset.mockResolvedValue({ reset: true });
  mocks.create.mockResolvedValue(pool);
  mocks.candidates.mockReturnValue({
    data: {
      pages: [
        {
          candidates: [
            {
              user_service_id: "member-id",
              slug: "My member",
              credential_binding: "user",
              protocol: "openai_completions",
              eligible: true,
              reason: null,
            },
          ],
        },
      ],
    },
    isLoading: false,
  });
});
describe("pool atomic editor", () => {
  it("keeps Save disabled until dirty, then sends contract and models in one revision-fenced update", async () => {
    const user = userEvent.setup();
    render(<PoolEditor pool={pool} onClose={vi.fn()} />);
    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
    expect(
      screen.queryByLabelText("Model for member 1"),
    ).not.toBeInTheDocument();
    await user.click(
      screen.getByRole("combobox", { name: "Request contract" }),
    );
    await user.click(screen.getByRole("option", { name: "AI chat" }));
    await user.type(screen.getByLabelText("Model for member 1"), "gpt-example");
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Save" })).toBeEnabled(),
    );
    expect(mocks.candidates).toHaveBeenLastCalledWith(
      expect.objectContaining({ contract: "ai_chat", peerIds: ["member-id"] }),
    );
    await user.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(mocks.update).toHaveBeenCalledTimes(1));
    expect(mocks.update).toHaveBeenCalledWith(
      expect.objectContaining({
        poolId: "pool-id",
        expected_revision: 17,
        member_contract: "ai_chat",
        description: null,
        failover: null,
        members: [
          expect.objectContaining({
            user_service_id: "member-id",
            model: "gpt-example",
            weight: 2,
          }),
        ],
      }),
    );
    expect(mocks.create).not.toHaveBeenCalled();
  });
  it("shows server eligibility reasons and loads additional pages explicitly", async () => {
    const more = vi.fn();
    mocks.candidates.mockReturnValue({
      data: {
        pages: [
          {
            candidates: [
              {
                user_service_id: "denied",
                slug: "Incompatible member",
                credential_binding: "platform",
                protocol: "anthropic_messages",
                eligible: false,
                reason: "incompatible_protocol",
              },
              {
                user_service_id: "declare",
                slug: "Custom member",
                credential_binding: "user",
                protocol: null,
                eligible: false,
                reason: "compatibility_declaration_required",
              },
            ],
          },
        ],
      },
      hasNextPage: true,
      fetchNextPage: more,
    });
    const user = userEvent.setup();
    render(<PoolEditor pool={pool} onClose={vi.fn()} />);
    expect(
      screen.getByText(
        "platform · anthropic_messages · Protocol is incompatible with this pool",
      ),
    ).toBeInTheDocument();
    const adds = screen.getAllByRole("button", { name: "Add" });
    expect(adds[0]).toBeDisabled();
    expect(adds[1]).toBeEnabled();
    await user.click(
      screen.getByRole("button", { name: "Load more candidates" }),
    );
    expect(more).toHaveBeenCalledTimes(1);
    await user.click(adds[1]!);
    expect(
      screen.getByLabelText("Confirm API compatibility for member 2"),
    ).not.toBeChecked();
  });
});

describe("pool management controls", () => {
  it("creates for the explicitly selected organization", async () => {
    const user = userEvent.setup();
    const change = vi.fn();
    const view = render(
      <ServicePoolsTab createOpen={false} onCreateOpenChange={change} />,
    );
    await user.click(screen.getByRole("combobox", { name: "Pool owner" }));
    await user.click(screen.getByRole("option", { name: "Research" }));
    expect(mocks.pools).toHaveBeenLastCalledWith("org-id");
    view.rerender(<ServicePoolsTab createOpen onCreateOpenChange={change} />);
    await user.type(screen.getByLabelText("Name"), "Research routing");
    await user.type(screen.getByLabelText("Slug"), "research-routing");
    await user.click(screen.getByRole("button", { name: "Add" }));
    await user.click(
      screen.getByLabelText("Confirm API compatibility for member 1"),
    );
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Save" })).toBeEnabled(),
    );
    await user.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() =>
      expect(mocks.create).toHaveBeenCalledWith(
        expect.objectContaining({
          org_id: "org-id",
          slug: "research-routing",
          members: [
            expect.objectContaining({
              user_service_id: "member-id",
              same_api_compatible: true,
            }),
          ],
        }),
      ),
    );
    expect(mocks.update).not.toHaveBeenCalled();
  });

  it("makes a nontext member toggle dirty and displays model and policy validation", async () => {
    const user = userEvent.setup();
    render(<PoolEditor pool={pool} onClose={vi.fn()} />);
    await user.click(screen.getByRole("switch", { name: "Member 1 enabled" }));
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Save" })).toBeEnabled(),
    );
    await user.click(
      screen.getByRole("combobox", { name: "Request contract" }),
    );
    await user.click(screen.getByRole("option", { name: "AI chat" }));
    expect(
      await screen.findByText("AI chat members require a model"),
    ).toBeVisible();
    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
    await user.type(
      screen.getByLabelText("Model for member 1"),
      "native-model",
    );
    await user.click(
      screen.getByRole("switch", { name: "Customize failover policy" }),
    );
    await user.clear(screen.getByLabelText("Maximum cooldown (ms)"));
    await user.type(screen.getByLabelText("Maximum cooldown (ms)"), "1");
    expect(
      await screen.findByText(
        "Maximum cooldown must be at least the base cooldown",
      ),
    ).toBeVisible();
    expect(screen.getByRole("button", { name: "Save" })).toBeDisabled();
    expect(mocks.update).not.toHaveBeenCalled();
  });

  it("inspects a non-root operation and updates draft declaration and strategy context", async () => {
    mocks.candidates.mockImplementation((query) => ({
      data: {
        pages: [
          {
            candidates: [
              {
                user_service_id: "operation-member",
                slug: "Items",
                credential_binding: "user",
                protocol: null,
                eligible: query.method === "GET" && query.path === "/items",
                reason:
                  query.method === "GET" && query.path === "/items"
                    ? null
                    : "operation_unsupported",
              },
            ],
          },
        ],
      },
    }));
    const user = userEvent.setup();
    render(<PoolEditor pool={pool} onClose={vi.fn()} />);
    expect(screen.getByRole("button", { name: "Add" })).toBeDisabled();
    await user.click(
      screen.getByRole("combobox", { name: "Candidate method" }),
    );
    await user.click(screen.getByRole("option", { name: "GET" }));
    await user.clear(screen.getByLabelText("Candidate operation path"));
    await user.type(
      screen.getByLabelText("Candidate operation path"),
      "/items",
    );
    await user.click(screen.getByRole("button", { name: "Add" }));
    await user.click(
      screen.getByLabelText("Confirm API compatibility for member 2"),
    );
    expect(mocks.candidates).toHaveBeenLastCalledWith(
      expect.objectContaining({
        method: "GET",
        path: "/items",
        peerIds: ["member-id", "operation-member"],
        declaredPeerIds: ["member-id", "operation-member"],
      }),
    );
    await user.click(screen.getByRole("combobox", { name: "Strategy" }));
    await user.click(screen.getByRole("option", { name: "Round Robin" }));
    expect(mocks.candidates).toHaveBeenLastCalledWith(
      expect.objectContaining({ strategy: "round_robin", declaredPeerIds: [] }),
    );
  });

  it("renders cooldown for the selected operation and resets a member or all members", async () => {
    mocks.health.mockReturnValue({
      data: {
        candidates: [
          {
            user_service_id: "member-id",
            slug: "My member",
            eligible: false,
            reason: "cooldown",
            credential_binding: "platform",
            consecutive_failures: 2,
            last_status: 429,
            cooldown_until: "2030-01-01T00:00:00Z",
          },
        ],
      },
    });
    const user = userEvent.setup();
    render(<PoolHealthDialog pool={pool} onClose={vi.fn()} />);
    expect(screen.getByText("Cooling down")).toBeVisible();
    expect(screen.getByText(/2 failures.*HTTP 429.*Retry after/)).toBeVisible();
    await user.click(screen.getByRole("combobox", { name: "Method" }));
    await user.click(screen.getByRole("option", { name: "GET" }));
    await user.clear(screen.getByLabelText("Operation path"));
    await user.type(screen.getByLabelText("Operation path"), "/items");
    expect(mocks.health).toHaveBeenLastCalledWith(
      expect.objectContaining({ method: "GET", path: "/items" }),
    );
    await user.click(screen.getByRole("button", { name: "Reset" }));
    expect(mocks.reset).toHaveBeenLastCalledWith({
      poolId: "pool-id",
      userServiceId: "member-id",
    });
    await user.click(
      screen.getByRole("button", { name: "Reset all cooldowns" }),
    );
    expect(mocks.reset).toHaveBeenLastCalledWith({
      poolId: "pool-id",
      userServiceId: undefined,
    });
  });
});

it("keeps labels from page two and saved IDs when a new candidate query resets pagination", async () => {
  mocks.health.mockReturnValue({
    data: {
      candidates: [
        { user_service_id: "member-id", slug: "Saved off-page member" },
      ],
    },
  });
  mocks.candidates.mockImplementation((query) => ({
    data: {
      pages: [
        { candidates: [] },
        ...(query.peerIds.includes("late-member") || query.search
          ? []
          : [
              {
                candidates: [
                  {
                    user_service_id: "late-member",
                    slug: "Readable second-page connection",
                    eligible: true,
                    reason: null,
                    credential_binding: "user",
                    protocol: null,
                  },
                ],
              },
            ]),
      ],
    },
  }));
  const user = userEvent.setup();
  render(<PoolEditor pool={pool} onClose={vi.fn()} />);
  expect(screen.getByText("Saved off-page member")).toBeVisible();
  await user.click(screen.getByRole("button", { name: "Add" }));
  expect(screen.getByText("Readable second-page connection")).toBeVisible();
  await user.type(
    screen.getByLabelText("Search candidate services"),
    "unrelated",
  );
  expect(screen.getByText("Readable second-page connection")).toBeVisible();
  expect(screen.getByText("Saved off-page member")).toBeVisible();
  expect(screen.queryByText(/Saved member;/)).not.toBeInTheDocument();
});

describe("pool member order in the routing editor", () => {
  it("saves a reordered route atomically with its original revision and preserves member settings", async () => {
    const user = userEvent.setup();
    render(
      <PoolEditor
        pool={{
          ...pool,
          members: [
            {
              user_service_id: "first",
              priority: 0,
              weight: 2,
              enabled: true,
              model: null,
              same_api_compatible: true,
            },
            {
              user_service_id: "second",
              priority: 10,
              weight: 5,
              enabled: false,
              model: null,
              same_api_compatible: true,
            },
          ],
        }}
        onClose={vi.fn()}
      />,
    );
    await user.click(screen.getByRole("button", { name: "Move member 2 up" }));
    await waitFor(() =>
      expect(screen.getByRole("button", { name: "Save" })).toBeEnabled(),
    );
    expect(mocks.update).not.toHaveBeenCalled();
    await user.click(screen.getByRole("button", { name: "Save" }));
    await waitFor(() => expect(mocks.update).toHaveBeenCalledTimes(1));
    expect(mocks.update).toHaveBeenCalledWith(
      expect.objectContaining({
        poolId: "pool-id",
        expected_revision: 17,
        strategy: "priority",
        members: [
          {
            user_service_id: "second",
            priority: 0,
            weight: 5,
            enabled: false,
            model: null,
            same_api_compatible: true,
          },
          {
            user_service_id: "first",
            priority: 1,
            weight: 2,
            enabled: true,
            model: null,
            same_api_compatible: true,
          },
        ],
      }),
    );
  });
});
