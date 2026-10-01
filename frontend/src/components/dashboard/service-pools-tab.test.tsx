import { fireEvent, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  PoolEditor,
  PoolHealthDialog,
  ServicePoolsTab,
} from "./service-pools-tab";
import type { PoolCandidate, ServicePool } from "@/schemas/pools";
const mocks = vi.hoisted(() => ({
  update: vi.fn(),
  create: vi.fn(),
  candidates: vi.fn(),
  health: vi.fn(),
  reset: vi.fn(),
  pools: vi.fn(),
  orgs: vi.fn(),
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
vi.mock("@/hooks/use-orgs", () => ({ useOrgs: mocks.orgs }));
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
  mocks.orgs.mockReturnValue({
    data: [{ id: "org-id", display_name: "Research", your_role: "admin" }],
  });
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

describe("pool atomic editor", () => {
  it("keeps displayed member numbers and edits aligned after priority reordering", async () => {
    const user = userEvent.setup();
    const second = {
      ...candidate,
      user_service_id: "second-id",
      name: "Second connection",
      slug: "second",
    };
    mocks.candidates.mockReturnValue(page([candidate, second]));
    render(
      <PoolEditor
        pool={{
          ...pool,
          members: [
            pool.members[0]!,
            {
              ...pool.members[0]!,
              user_service_id: second.user_service_id,
              priority: 1,
            },
          ],
        }}
        onClose={vi.fn()}
      />,
    );
    fireEvent.change(screen.getByLabelText("Priority for member 1"), {
      target: { value: "2" },
    });
    expect(screen.getByLabelText("Priority for member 1")).toHaveValue(1);
    expect(screen.getAllByRole("spinbutton")[0]).toBe(
      screen.getByLabelText("Priority for member 1"),
    );
    await user.click(screen.getByRole("switch", { name: "Member 1 enabled" }));
    await user.click(saveButton());
    expect(mocks.update).toHaveBeenCalledWith(
      expect.objectContaining({
        members: [
          expect.objectContaining({
            user_service_id: "member-id",
            priority: 2,
            enabled: true,
          }),
          expect.objectContaining({
            user_service_id: "second-id",
            priority: 1,
            enabled: false,
          }),
        ],
      }),
    );
  });
  it("enables Save after a single pasted name edit in a valid existing pool", async () => {
    const user = userEvent.setup();
    render(<PoolEditor pool={pool} onClose={vi.fn()} />);
    expect(saveButton()).toBeDisabled();
    fireEvent.change(screen.getByLabelText("Name"), {
      target: { value: "Renamed in one event" },
    });
    await waitFor(() => expect(saveButton()).toBeEnabled());
    await user.click(saveButton());
    expect(mocks.update).toHaveBeenCalledWith(
      expect.objectContaining({
        name: "Renamed in one event",
        expected_revision: 17,
        poolId: "pool-id",
      }),
    );
  });
  it("atomically saves AI contract and required models, then clears models when switching to same API", async () => {
    const user = userEvent.setup();
    const view = render(<PoolEditor pool={pool} onClose={vi.fn()} />);
    await user.click(screen.getByRole("button", { name: /^AI chat/ }));
    expect(saveButton()).toBeDisabled();
    await user.type(screen.getByLabelText("Model for member 1"), "gpt-example");
    await waitFor(() => expect(saveButton()).toBeEnabled());
    await user.click(saveButton());
    expect(mocks.update).toHaveBeenCalledWith(
      expect.objectContaining({
        expected_revision: 17,
        member_contract: "ai_chat",
        members: [expect.objectContaining({ model: "gpt-example", weight: 2 })],
      }),
    );
    view.unmount();
    mocks.update.mockClear();
    render(
      <PoolEditor
        pool={{
          ...pool,
          member_contract: "ai_chat",
          members: [{ ...pool.members[0]!, model: "old-model" }],
        }}
        onClose={vi.fn()}
      />,
    );
    await user.click(screen.getByRole("button", { name: /^Same API/ }));
    expect(
      screen.queryByLabelText("Model for member 1"),
    ).not.toBeInTheDocument();
    await user.click(saveButton());
    expect(mocks.update).toHaveBeenCalledWith(
      expect.objectContaining({
        member_contract: "same_api",
        members: [expect.objectContaining({ model: null })],
      }),
    );
  });
  it("browses inventory without a path and applies only validated complete operations", async () => {
    const user = userEvent.setup();
    render(<PoolEditor pool={pool} onClose={vi.fn()} />);
    expect(mocks.candidates.mock.calls[0]![0]).toMatchObject({
      checkOperation: false,
    });
    await user.click(screen.getByText("Check an operation (optional)"));
    await user.type(screen.getByLabelText("Operation path"), "test/");
    expect(screen.queryByRole("alert")).not.toBeInTheDocument();
    expect(
      mocks.candidates.mock.calls.every(([q]) => q.path === undefined),
    ).toBe(true);
    await user.click(screen.getByRole("button", { name: "Check operation" }));
    expect(screen.getByRole("alert")).toHaveTextContent(
      "Remove the trailing slash",
    );
    expect(
      mocks.candidates.mock.calls.every(([q]) => q.path === undefined),
    ).toBe(true);
    await user.clear(screen.getByLabelText("Operation path"));
    await user.type(screen.getByLabelText("Operation path"), "/items");
    await user.click(screen.getByRole("combobox", { name: "Method" }));
    await user.click(screen.getByRole("option", { name: "GET" }));
    await user.click(screen.getByRole("button", { name: "Check operation" }));
    expect(mocks.candidates).toHaveBeenCalledWith(
      expect.objectContaining({
        checkOperation: true,
        method: "GET",
        path: "/items",
      }),
    );
    await user.click(
      screen.getByRole("button", { name: "Clear operation check" }),
    );
    expect(mocks.candidates.mock.calls.at(-1)![0].checkOperation).toBe(false);
  });
  it("keeps compatibility declarations available after search hides the selected connections", async () => {
    const custom = {
      ...candidate,
      user_service_id: "custom-id",
      name: "Custom connection",
      slug: "custom",
      catalog_service_id: null,
      requires_compatibility_declaration: true,
      eligible: false,
      reason: "compatibility_declaration_required",
    };
    mocks.candidates.mockImplementation((q) =>
      q.selectedOnly
        ? page(
            [candidate, custom]
              .filter((row) => q.peerIds.includes(row.user_service_id))
              .map((row) => ({
                ...row,
                requires_compatibility_declaration:
                  q.peerIds.includes("custom-id"),
                reason: "compatibility_declaration_required",
              })),
          )
        : page(q.search ? [] : [custom]),
    );
    const user = userEvent.setup();
    render(
      <PoolEditor
        pool={{
          ...pool,
          members: [{ ...pool.members[0]!, same_api_compatible: false }],
        }}
        onClose={vi.fn()}
      />,
    );
    await user.click(screen.getByRole("button", { name: "Add" }));
    await user.type(
      screen.getByRole("textbox", { name: "Search candidate services" }),
      "hide selected",
    );
    await waitFor(() =>
      expect(
        screen.getByText("No connections match this search."),
      ).toBeVisible(),
    );
    for (const n of [1, 2]) {
      const checkbox = screen.getByLabelText(
        `Confirm API compatibility for member ${n}`,
      );
      expect(checkbox).toBeVisible();
      await user.click(checkbox);
      expect(checkbox).toBeChecked();
    }
    await user.click(saveButton());
    expect(mocks.update).toHaveBeenCalledWith(
      expect.objectContaining({
        members: [
          expect.objectContaining({ same_api_compatible: true }),
          expect.objectContaining({ same_api_compatible: true, priority: 1 }),
        ],
      }),
    );
  });
  it("preserves paging and displays readable reasons for unavailable connections", async () => {
    const more = vi.fn();
    mocks.candidates.mockReturnValue({
      ...page([
        {
          ...candidate,
          user_service_id: "bad",
          name: "Different API",
          reason: "incompatible_protocol",
          eligible: false,
        },
      ]),
      hasNextPage: true,
      fetchNextPage: more,
    });
    const user = userEvent.setup();
    render(<PoolEditor pool={pool} onClose={vi.fn()} />);
    expect(
      screen.getByText("Protocol is incompatible with this pool"),
    ).toBeVisible();
    expect(screen.getByRole("button", { name: "Add" })).toBeDisabled();
    await user.click(
      screen.getByRole("button", { name: "Load more connections" }),
    );
    expect(more).toHaveBeenCalledTimes(1);
  });
  it("makes switches dirty and validates advanced policy before saving", async () => {
    const user = userEvent.setup();
    render(<PoolEditor pool={pool} onClose={vi.fn()} />);
    await user.click(screen.getByRole("switch", { name: "Member 1 enabled" }));
    await waitFor(() => expect(saveButton()).toBeEnabled());
    await user.click(screen.getByText("Advanced settings"));
    await user.click(
      screen.getByRole("switch", { name: "Customize retry settings" }),
    );
    await user.clear(screen.getByLabelText("Maximum cooldown (ms)"));
    await user.type(screen.getByLabelText("Maximum cooldown (ms)"), "1");
    expect(
      await screen.findByText(
        "Maximum cooldown must be at least the base cooldown",
      ),
    ).toBeVisible();
    expect(saveButton()).toBeDisabled();
    await user.click(screen.getByRole("combobox", { name: "Routing" }));
    await user.click(screen.getByRole("option", { name: "Round robin" }));
    await user.click(saveButton());
    expect(mocks.update).toHaveBeenCalledWith(
      expect.objectContaining({
        strategy: "round_robin",
        failover: null,
        member_contract: "same_api",
        members: [expect.objectContaining({ model: null, priority: 0 })],
      }),
    );
  });
  it("keeps a conflicting draft visible without retrying or discarding changes", async () => {
    mocks.update.mockRejectedValue(
      new Error("Pool configuration changed; reload before saving"),
    );
    const user = userEvent.setup();
    render(<PoolEditor pool={pool} onClose={vi.fn()} />);
    await user.type(screen.getByLabelText("Name"), " edited");
    await user.click(saveButton());
    expect(
      await screen.findByText(
        "Pool configuration changed; reload before saving",
      ),
    ).toBeVisible();
    expect(screen.getByLabelText("Name")).toHaveValue("My pool edited");
    expect(mocks.update).toHaveBeenCalledTimes(1);
  });
});

describe("pool management", () => {
  it("keeps actions nonmodal and releases dialog pointer control after repeated cancellation", async () => {
    const user = userEvent.setup();
    mocks.pools.mockReturnValue({ data: [pool], isLoading: false });
    render(<ServicePoolsTab createOpen={false} onCreateOpenChange={vi.fn()} />);
    const trigger = screen.getAllByRole("button", {
      name: "Actions for My pool",
    })[0]!;
    for (let attempt = 0; attempt < 3; attempt += 1) {
      await user.click(trigger);
      expect(await screen.findByRole("menu")).toBeVisible();
      expect(document.body).not.toHaveStyle({ pointerEvents: "none" });
      await user.click(screen.getByRole("menuitem", { name: "Delete" }));
      expect(await screen.findByRole("dialog")).toHaveTextContent(
        "Delete My pool?",
      );
      expect(document.body).toHaveStyle({ pointerEvents: "none" });
      await user.click(screen.getByRole("button", { name: "Cancel" }));
      await waitFor(() => {
        expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
        expect(document.body).not.toHaveStyle({ pointerEvents: "none" });
        expect(trigger).toHaveFocus();
      });
    }
  });
  it.each(["Use pool", "Edit", "Connections & health", "Delete"])(
    "hands keyboard focus from %s to its dialog and back to Actions on Escape",
    async (action) => {
      const user = userEvent.setup();
      mocks.pools.mockReturnValue({ data: [pool], isLoading: false });
      render(
        <ServicePoolsTab createOpen={false} onCreateOpenChange={vi.fn()} />,
      );
      const trigger = screen.getAllByRole("button", {
        name: "Actions for My pool",
      })[0]!;
      trigger.focus();
      await user.keyboard("{Enter}");
      await screen.findByRole("menu");
      const steps = [
        "Use pool",
        "Edit",
        "Connections & health",
        "Disable",
        "Delete",
      ].indexOf(action);
      await user.keyboard("{Home}" + "{ArrowDown}".repeat(steps));
      expect(screen.getByRole("menuitem", { name: action })).toHaveFocus();
      await user.keyboard("{Enter}");
      await waitFor(() =>
        expect(screen.getByRole("dialog")).toContainElement(
          document.activeElement as HTMLElement,
        ),
      );
      await user.keyboard("{Tab}");
      expect(screen.getByRole("dialog")).toContainElement(
        document.activeElement as HTMLElement,
      );
      await user.keyboard("{Escape}");
      await waitFor(() => {
        expect(screen.queryByRole("dialog")).not.toBeInTheDocument();
        expect(trigger).toHaveFocus();
      });
      await user.keyboard("{Enter}");
      expect(await screen.findByRole("menu")).toBeVisible();
      await user.keyboard("{Escape}");
    },
  );
  it("returns focus to a directly clicked pool name after another pool's menu was used", async () => {
    const user = userEvent.setup();
    mocks.pools.mockReturnValue({
      data: [pool, { ...pool, id: "other-id", name: "Other pool" }],
      isLoading: false,
    });
    render(<ServicePoolsTab createOpen={false} onCreateOpenChange={vi.fn()} />);
    const previousTrigger = screen.getAllByRole("button", {
      name: "Actions for My pool",
    })[0]!;
    await user.click(previousTrigger);
    await user.click(screen.getByRole("menuitem", { name: "Delete" }));
    await user.click(screen.getByRole("button", { name: "Cancel" }));
    await waitFor(() => expect(previousTrigger).toHaveFocus());

    const nameTrigger = screen.getByRole("button", { name: "Other pool" });
    await user.click(nameTrigger);
    expect(await screen.findByRole("dialog")).toHaveTextContent(
      "Edit service pool",
    );
    await user.keyboard("{Escape}");
    await waitFor(() => expect(nameTrigger).toHaveFocus());
  });
  it("uses the selected owner, generates a slug, and leaves the create trigger to the page header", async () => {
    const user = userEvent.setup();
    const change = vi.fn();
    const view = render(
      <ServicePoolsTab createOpen={false} onCreateOpenChange={change} />,
    );
    expect(
      screen.queryByRole("button", { name: /create pool/i }),
    ).not.toBeInTheDocument();
    await user.click(screen.getByRole("combobox", { name: "Owner" }));
    await user.click(screen.getByRole("option", { name: "Research" }));
    expect(mocks.pools).toHaveBeenLastCalledWith("org-id");
    view.rerender(<ServicePoolsTab createOpen onCreateOpenChange={change} />);
    await user.type(screen.getByLabelText("Name"), "Research routing");
    expect(screen.getByLabelText("Pool slug")).toHaveValue("research-routing");
    await user.click(screen.getByRole("button", { name: "Add" }));
    await user.click(screen.getByRole("button", { name: "Create pool" }));
    expect(mocks.create).toHaveBeenCalledWith(
      expect.objectContaining({ org_id: "org-id", slug: "research-routing" }),
    );
  });
  it("keeps generated slugs valid when a separator falls at the length limit", async () => {
    render(<PoolEditor onClose={vi.fn()} />);
    fireEvent.change(screen.getByLabelText("Name"), {
      target: { value: `${"a".repeat(79)} backup` },
    });
    await waitFor(() =>
      expect(screen.getByLabelText("Pool slug")).toHaveValue("a".repeat(79)),
    );
    expect(screen.getByRole("button", { name: "Create pool" })).toBeEnabled();
  });

  it("hides the owner selector without manageable organizations", () => {
    mocks.orgs.mockReturnValue({
      data: [{ id: "viewer", your_role: "viewer" }],
    });
    render(<ServicePoolsTab createOpen={false} onCreateOpenChange={vi.fn()} />);
    expect(
      screen.queryByRole("combobox", { name: "Owner" }),
    ).not.toBeInTheDocument();
    expect(screen.getByText(/your personal connections/)).toBeVisible();
  });
  it("shows health only after an operation check, and resets a member or all members", async () => {
    mocks.health.mockImplementation((q) => ({
      data: {
        operation_checked: q.checkOperation,
        candidates: [
          {
            ...candidate,
            ...(q.checkOperation
              ? {
                  reason: "cooldown",
                  eligible: false,
                  consecutive_failures: 2,
                  last_status: 429,
                  cooldown_until: "2030-01-01T00:00:00Z",
                }
              : {}),
          },
        ],
      },
    }));
    const user = userEvent.setup();
    render(<PoolHealthDialog pool={pool} onClose={vi.fn()} />);
    expect(screen.getByText("Available connection")).toBeVisible();
    // Badge renders a div, which must not be nested in a paragraph.
    expect(screen.getByText("Available connection").closest("p")).toBeNull();
    await user.type(screen.getByLabelText("Operation path"), "test/");
    expect(
      mocks.health.mock.calls.every(([q]) => q.checkOperation === false),
    ).toBe(true);
    await user.click(screen.getByRole("button", { name: "Check operation" }));
    expect(screen.getByRole("alert")).toHaveTextContent(
      "Remove the trailing slash",
    );
    await user.clear(screen.getByLabelText("Operation path"));
    await user.type(screen.getByLabelText("Operation path"), "/items");
    await user.click(screen.getByRole("button", { name: "Check operation" }));
    expect(screen.getByText("Cooling down")).toBeVisible();
    expect(screen.getByText(/2 failures.*HTTP 429.*Retry after/)).toBeVisible();
    await user.click(screen.getByRole("button", { name: "Reset" }));
    expect(mocks.reset).toHaveBeenCalledWith({
      poolId: "pool-id",
      userServiceId: "member-id",
    });
    await user.click(
      screen.getByRole("button", { name: "Reset all cooldowns" }),
    );
    expect(mocks.reset).toHaveBeenCalledWith({
      poolId: "pool-id",
      userServiceId: undefined,
    });
  });
  it("does not offer cooldown reset for legacy routing", () => {
    render(
      <PoolHealthDialog
        pool={{ ...pool, strategy: "weighted" }}
        onClose={vi.fn()}
      />,
    );
    expect(
      screen.queryByRole("button", { name: /Reset/ }),
    ).not.toBeInTheDocument();
    expect(screen.getByText(/does not use cooldowns/)).toBeVisible();
  });
});
