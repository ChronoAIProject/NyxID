import {
  act,
  fireEvent,
  render,
  screen,
  waitFor,
  within,
} from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import { useLayoutEffect } from "react";
import { useServiceGroupOrder } from "./use-service-group-order";
import { ServiceOrderActions } from "@/components/dashboard/service-order-actions";
import { ServiceConnectionTable } from "@/components/dashboard/service-connection-table";
import { ServiceAgentOrderPanel } from "@/components/dashboard/service-agent-order-panel";
import { groupServiceConnections } from "@/lib/service-groups";
import { useAuthStore } from "@/stores/auth-store";
import type { User } from "@/types/api";
import type { KeyInfo } from "@/types/keys";
import { ApiError } from "@/lib/api-client";
const catalog = "aaaaaaaa-aaaa-5aaa-8aaa-aaaaaaaaaaaa";
const group = `catalog:${catalog}`;
const { state, save, release, refetchPreference, refetchKeys } = vi.hoisted(
  () => ({
    state: {
      inventory: [] as KeyInfo[],
      fetching: false,
      error: false,
      stamp: 1,
      preference: {
        groups: [] as { group: string; ordered: string[] }[],
        version: 1,
        updated_at: null,
      },
      pending: false,
      preferenceError: false,
      unavailable: false,
    },
    save: vi.fn(),
    release: vi.fn(),
    refetchPreference: vi.fn(),
    refetchKeys: vi.fn(),
  }),
);
vi.mock("@/hooks/use-keys", () => ({
  useKeys: () => ({
    data: state.error ? undefined : state.inventory,
    isError: state.error,
    isFetching: state.fetching,
    isLoading: false,
    dataUpdatedAt: state.stamp,
    refetch: refetchKeys,
  }),
}));
vi.mock("@/hooks/use-service-preference", () => ({
  SERVICE_ORDER_UNAVAILABLE: "unavailable",
  useServicePreference: () => ({
    data: state.unavailable ? "unavailable" : state.preference,
    isError: state.preferenceError,
    isFetching: state.pending,
    isLoading: false,
    refetch: refetchPreference,
  }),
  useSaveServiceGroupOrder: () => ({ save, release, isPending: false }),
}));
vi.mock("@tanstack/react-router", () => ({
  Link: ({ children, ...props }: { children: React.ReactNode }) => (
    <a {...props}>{children}</a>
  ),
}));
vi.mock("@/hooks/use-service-insights", () => ({
  useServiceInsights: () => ({
    status: "ready",
    connections: new Map(),
    refresh: vi.fn(),
  }),
}));
function key(i: number): KeyInfo {
  return {
    id: `${String(i + 1).padStart(8, "0")}-1111-4111-8111-111111111111`,
    slug: `connection-${i}`,
    label: "Anthropic",
    catalog_service_id: catalog,
    catalog_service_slug: "llm-anthropic",
    catalog_service_name: "Anthropic",
    is_active: i < 4,
    service_type: "http",
    status: "active",
    credential_source: { type: "personal" },
    auto_connected: false,
    endpoint_url: "https://example.com",
    credential_type: "bearer",
    auth_method: "bearer",
    node_id: null,
    ws_frame_injections: [],
    created_at: "2026-10-07",
  } as unknown as KeyInfo;
}
let latest: ReturnType<typeof useServiceGroupOrder>;
function Harness({
  table = false,
  sticky = false,
  form = false,
}: {
  table?: boolean;
  sticky?: boolean;
  form?: boolean;
}) {
  const order = useServiceGroupOrder(state.error ? [] : state.inventory);
  useLayoutEffect(() => {
    latest = order;
  }, [order]);
  const entry = groupServiceConnections(order.inventory)[0]!;
  return (
    <>
      <button onClick={() => order.start(entry)}>Start</button>
      <button
        onClick={() =>
          order.update([...order.connections].reverse().map((key) => key.id))
        }
      >
        Reverse
      </button>
      <button onClick={() => void order.retrySave()}>Submit</button>
      <button onClick={() => void order.recover(true)}>Overwrite</button>
      <button onClick={() => order.guard()}>Navigate</button>
      <output>
        {order.groupId ?? "closed"}|{order.busy ? "busy" : "idle"}|
        {order.ordered.join(",")}
      </output>
      {sticky && order.groupId && (
        <ServiceOrderActions order={order} formId="test-order" />
      )}
      {form && order.groupId ? (
        <div data-testid="order-form">
          {order.validationError && (
            <p role="alert">{order.validationError}</p>
          )}
          <button type="button" onClick={order.reset}>
            Reset to default
          </button>
          <form
            id="test-order"
            aria-label="Agent order for Anthropic"
            onSubmit={(event) => void order.save(event)}
          />
        </div>
      ) : table && order.groupId ? (
        <ServiceConnectionTable
          connections={order.connections}
          ordering={order}
          serviceName="Anthropic"
          orderFormId={sticky ? "test-order" : undefined}
          externalOrderActions={sticky}
        />
      ) : (
        <ServiceAgentOrderPanel group={entry} order={order} actions={null} />
      )}
    </>
  );
}
beforeEach(() => {
  vi.clearAllMocks();
  state.inventory = Array.from({ length: 3 }, (_, i) => key(i));
  state.error = false;
  state.fetching = false;
  state.pending = false;
  state.unavailable = false;
  state.preferenceError = false;
  state.stamp = 1;
  state.preference = {
    groups: [{ group, ordered: [key(0).id] }],
    version: 1,
    updated_at: null,
  };
  useAuthStore.setState({ user: { id: "one" } as User });
  save.mockResolvedValue({ groups: [], version: 2, updated_at: null });
  refetchPreference.mockResolvedValue({
    data: state.preference,
    isError: false,
  });
  refetchKeys.mockImplementation(async () => ({
    data: state.inventory,
    isError: false,
  }));
  window.confirm = vi.fn(() => true);
});
it("reconciles only successful inventory snapshots, preserves draft/version and adds New metadata", async () => {
  const { rerender } = render(<Harness />);
  fireEvent.click(screen.getByText("Start"));
  fireEvent.click(screen.getByText("Reverse"));
  const before = latest.ordered;
  state.error = true;
  state.inventory = [];
  rerender(<Harness />);
  expect(latest.connections.map((k) => k.id)).toEqual(before);
  expect(latest.blocked).toBe(true);
  state.error = false;
  state.inventory = [key(0), key(2), key(3)];
  state.stamp++;
  rerender(<Harness />);
  await waitFor(() =>
    expect(latest.connections.map((k) => k.id)).toEqual([
      key(2).id,
      key(0).id,
      key(3).id,
    ]),
  );
  expect(latest.newIds).toEqual([key(3).id]);
  expect(latest.dirty).toBe(true);
  await act(() => latest.retrySave());
  expect(save).toHaveBeenCalledWith(group, {
    ordered: [key(2).id, key(0).id, key(3).id],
    expected_version: 1,
  });
});
it("abandons deferred recovery on identity switch and permits new identity editing/navigation", async () => {
  let resolve!: (value: unknown) => void;
  refetchPreference.mockImplementation(
    () =>
      new Promise((r) => {
        resolve = r;
      }),
  );
  const { rerender } = render(<Harness />);
  fireEvent.click(screen.getByText("Start"));
  fireEvent.click(screen.getByText("Reverse"));
  fireEvent.click(screen.getByText("Overwrite"));
  expect(latest.busy).toBe(true);
  act(() => useAuthStore.setState({ user: { id: "two" } as User }));
  rerender(<Harness />);
  expect(latest.groupId).toBeUndefined();
  expect(latest.busy).toBe(false);
  expect(latest.guard()).toBe(true);
  fireEvent.click(screen.getByText("Start"));
  fireEvent.click(screen.getByText("Reverse"));
  await act(async () => resolve({ data: state.preference, isError: false }));
  expect(save).not.toHaveBeenCalled();
  expect(latest.groupId).toBe(group);
  expect(latest.busy).toBe(false);
});
it("discard is idempotent so tab plus router blocker asks once, cancellation keeps draft", () => {
  render(<Harness />);
  fireEvent.click(screen.getByText("Start"));
  fireEvent.click(screen.getByText("Reverse"));
  vi.mocked(window.confirm).mockReturnValueOnce(false);
  expect(latest.guard()).toBe(false);
  expect(latest.groupId).toBe(group);
  vi.mocked(window.confirm).mockClear();
  act(() => {
    expect(latest.guard()).toBe(true);
    expect(latest.guard()).toBe(true);
  });
  expect(window.confirm).toHaveBeenCalledTimes(1);
});
it("validates all 201 IDs in the real order form without a PUT and permits confirmed reset", async () => {
  state.inventory = Array.from({ length: 201 }, (_, i) => key(i));
  const originalIds = state.inventory.map((connection) => connection.id);
  render(<Harness form sticky />);
  fireEvent.click(screen.getByText("Start"));
  expect(latest.connections.map((connection) => connection.id)).toEqual(
    originalIds,
  );
  expect(latest.inventory.map((connection) => connection.id)).toEqual(
    originalIds,
  );
  const form = document.querySelector("form")!;
  const formRoot = form.parentElement!;
  const movedIds = [
    originalIds[1]!,
    originalIds[0]!,
    ...originalIds.slice(2),
  ];
  act(() => latest.update(movedIds));
  expect(latest.ordered).toEqual(movedIds);
  expect(latest.connections.map((connection) => connection.id)).toEqual(
    movedIds,
  );
  const stickySave = document.querySelector<HTMLButtonElement>(
    'button[form="test-order"]',
  )!;
  expect(stickySave.form).toBe(form);
  expect(form.elements).toContain(stickySave);
  expect(stickySave).toBeEnabled();
  fireEvent.click(stickySave);
  await waitFor(
    () => {
      const alert = formRoot.querySelector("[role=alert]");
      expect(alert).toBeVisible();
      expect(alert).toHaveTextContent(/200/);
      expect(formRoot.querySelectorAll('[role="alert"]')).toHaveLength(1);
      expect(
        within(formRoot).getAllByText(latest.validationError!, { exact: true }),
      ).toHaveLength(1);
      expect(latest.submitCount).toBe(1);
    },
    { container: formRoot },
  );
  expect(latest.ordered).toEqual(movedIds);
  expect(save).not.toHaveBeenCalled();
  fireEvent.click(
    within(formRoot).getByRole("button", { name: "Reset to default" }),
  );
  expect(window.confirm).toHaveBeenCalledExactlyOnceWith(
    "Reset this service's agent order to default server discovery order?",
  );
  expect(latest.connections.map((connection) => connection.id)).toEqual(
    originalIds,
  );
  await waitFor(() => {
    expect(formRoot.querySelector("[role=alert]")).toBeNull();
    expect(stickySave).toBeEnabled();
  });
  expect(save).not.toHaveBeenCalled();
  fireEvent.click(stickySave);
  await waitFor(() =>
    expect(save).toHaveBeenCalledExactlyOnceWith(group, {
      ordered: [],
      expected_version: 1,
    }),
  );
  expect(latest.inventory.map((connection) => connection.id)).toEqual(
    originalIds,
  );
});
it("external sticky Save submits the real order form while connection rows stay outside it", async () => {
  const originalIds = state.inventory.map((connection) => connection.id);
  render(<Harness table sticky />);
  fireEvent.click(screen.getByText("Start"));
  const form = document.querySelector("form")!;
  const tableRoot = form.parentElement!;
  expect(tableRoot.querySelectorAll("[data-ordering-row]")).toHaveLength(3);
  expect(tableRoot.querySelector("table")).not.toBeNull();
  expect(form.querySelector("table")).toBeNull();
  const stickySave = document.querySelector<HTMLButtonElement>(
    'button[form="test-order"]',
  )!;
  expect(stickySave.form).toBe(form);
  expect(form.elements).toContain(stickySave);
  expect(stickySave).toBeDisabled();
  const row = within(
    tableRoot.querySelector(
      `[data-ordering-row="${key(1).id}"]`,
    )! as HTMLElement,
  );
  fireEvent.click(
    row.getByRole("button", { name: "Move Anthropic (connection-1) up" }),
  );
  expect(stickySave).toBeEnabled();
  fireEvent.click(stickySave);
  await waitFor(() =>
    expect(save).toHaveBeenCalledExactlyOnceWith(group, {
      ordered: [originalIds[1], originalIds[0], originalIds[2]],
      expected_version: 1,
    }),
  );
});
it("unknown read states preserve known keys pills and never claim absent order", () => {
  state.unavailable = true;
  const { rerender } = render(<Harness />);
  expect(screen.getByText(/Saved agent order unknown/)).toBeVisible();
  expect(screen.queryByText("Default server discovery order")).toBeNull();
  state.inventory[0] = { ...key(0), preference_rank: 1 };
  rerender(<Harness />);
  const summary = within(
    screen.getByRole("region", { name: "Agent discovery order for Anthropic" }),
  );
  expect(summary.getByText("Preferred in discovery:")).toBeVisible();
  expect(summary.getByText("Anthropic", { exact: true })).toBeVisible();
  expect(summary.getByRole("status")).toHaveTextContent(
    "Saving agent order requires the backend update.",
  );
  state.unavailable = false;
  state.pending = true;
  rerender(<Harness />);
  expect(summary.getByText("Preferred in discovery:")).toBeVisible();
  expect(summary.getByRole("status")).toHaveTextContent("Loading agent order.");
  expect(summary.getAllByText("Loading agent order.")).toHaveLength(1);
  state.pending = false;
  state.preferenceError = true;
  rerender(<Harness />);
  expect(summary.getByText("Preferred in discovery:")).toBeVisible();
  expect(summary.getByRole("alert")).toHaveTextContent(
    "Agent order could not be loaded.",
  );
  expect(summary.getAllByText("Agent order could not be loaded.")).toHaveLength(1);
  state.preferenceError = false;
  state.pending = true;
  state.inventory[0] = key(0);
  rerender(<Harness />);
  expect(summary.getByRole("status")).toHaveTextContent("Loading agent order.");
  expect(summary.getAllByText("Loading agent order.")).toHaveLength(1);
  expect(screen.queryByText("Default server discovery order")).toBeNull();
});
it("explains HTTP eligibility and retains protocol-specific saved positions without inventing SSH discovery", () => {
  state.inventory = [
    key(0),
    {
      ...key(1),
      label: "Shell connection",
      slug: "ssh-server",
      service_type: "ssh",
      preference_position: 1,
    },
    key(2),
  ];
  state.preference.groups = [
    { group, ordered: [key(1).id, key(2).id, key(0).id] },
  ];
  const { rerender } = render(<Harness table />);
  const help = screen.getByRole("button", { name: "How discovery order works" });
  expect(screen.queryByRole("tooltip")).toBeNull();
  fireEvent.click(help);
  expect(
    within(screen.getByRole("tooltip")).getByText(
      "Agents see only the enabled HTTP connections they can access.",
    ),
  ).toBeVisible();
  expect(screen.queryByText("slug__…")).toBeNull();
  expect(screen.queryByText("ssh-server__…")).toBeNull();
  expect(
    within(screen.getByRole("tooltip")).getByText(/Other protocol connections keep their saved positions/),
  ).toHaveTextContent("excluded from tool discovery");
  fireEvent.click(screen.getByText("Start"));
  const ssh = document.querySelector(
    `[data-service-connection-row="${key(1).id}"]`,
  )! as HTMLElement;
  expect(
    within(ssh).getByLabelText(/Saved order position 1/),
  ).toHaveTextContent("Saved #1 · SSH");
  expect(within(ssh).queryByLabelText(/Discovery preference/)).toBeNull();
  const http = document.querySelector(
    `[data-service-connection-row="${key(2).id}"]`,
  )! as HTMLElement;
  expect(
    within(http).getByLabelText(/Discovery preference 1 for Anthropic/),
  ).toHaveTextContent("Discovery #1");
  fireEvent.click(within(ssh).getByRole("button", { name: /^Move .* down$/ }));
  expect(
    within(ssh).getByLabelText(/Saved order position 2/),
  ).toHaveTextContent("Saved #2 · SSH");
  expect(
    within(http).getByLabelText(/Discovery preference 1 for Anthropic/),
  ).toHaveTextContent("Discovery #1");
  fireEvent.click(screen.getByRole("button", { name: "Cancel" }));
  rerender(
    <>
      <Harness />
      <ServiceConnectionTable
        connections={state.inventory}
        serviceName="Anthropic"
      />
    </>,
  );
  const savedSsh = document.querySelector(
    `[data-service-connection-row="${key(1).id}"]`,
  )! as HTMLElement;
  expect(
    within(savedSsh).getByLabelText(/Saved order position 1/),
  ).toHaveTextContent("Saved #1 · SSH");
  expect(within(savedSsh).queryByLabelText(/Discovery preference/)).toBeNull();
  expect(screen.queryByText("ssh-server__…")).toBeNull();
});
it("validates retry and overwrite after a successful inventory refresh grows beyond 200", async () => {
  save.mockRejectedValueOnce(new Error("network failure"));
  const { rerender } = render(<Harness />);
  fireEvent.click(screen.getByText("Start"));
  fireEvent.click(screen.getByText("Reverse"));
  await act(() => latest.retrySave());
  expect(latest.failure).toBe("network");
  state.inventory = Array.from({ length: 201 }, (_, i) => key(i));
  state.stamp++;
  rerender(<Harness />);
  expect(latest.connections).toHaveLength(201);
  await act(() => latest.retrySave());
  expect(latest.validationError).toBeTruthy();
  expect(save).toHaveBeenCalledTimes(1);
  await act(() => latest.recover(true));
  expect(latest.validationError).toBeTruthy();
  expect(save).toHaveBeenCalledTimes(1);
  expect(latest.connections).toHaveLength(201);
  expect(latest.dirty).toBe(true);
});
it("capacity release confirms, keeps draft, handles CAS and uses released version", async () => {
  save.mockRejectedValueOnce(
    new ApiError(400, {
      message:
        "Validation error: Agent order storage is full (200 connections across all services). Reset the agent order of another service, or release unavailable preferences for services you can no longer access, then try again.",
      error: "validation",
      error_code: 1000,
    }),
  );
  release.mockResolvedValue({ groups: [], version: 3, updated_at: null });
  const { rerender } = render(<Harness table />);
  fireEvent.click(screen.getByText("Start"));
  fireEvent.click(screen.getByText("Reverse"));
  await act(() => latest.retrySave());
  expect(latest.failure).toBe("capacity");
  expect(
    screen.getAllByText(/^Validation error: Agent order storage is full/),
  ).toHaveLength(1);
  expect(
    screen.getByText(/^Validation error: Agent order storage is full/),
  ).toBeVisible();
  vi.mocked(window.confirm).mockReturnValueOnce(false);
  await act(() => latest.releaseHidden());
  expect(release).not.toHaveBeenCalled();
  await act(() => latest.releaseHidden());
  expect(latest.groupId).toBe(group);
  expect(latest.readyToRetry).toBe(true);
  state.inventory = [...state.inventory, key(3)];
  state.stamp++;
  rerender(<Harness table />);
  expect(latest.message).toContain("New connections were appended");
  expect(screen.getByRole("button", { name: "Retry save" })).toBeVisible();
  fireEvent.click(screen.getByRole("button", { name: "Retry save" }));
  await waitFor(() => expect(save).toHaveBeenCalledTimes(2));
  expect(save.mock.calls[1]?.[1].expected_version).toBe(3);
});

it("explains real provider groups with null inference without guessing a provider slug", () => {
  state.inventory = state.inventory.map((key) => ({ ...key, inference: null }));
  const { rerender } = render(<Harness />);
  fireEvent.click(screen.getByRole("button", { name: "How discovery order works" }));
  expect(
    within(screen.getByRole("tooltip")).getByText("Provider gateway routing is separate from this discovery order."),
  ).toBeVisible();
  state.inventory = state.inventory.map((key) => ({
    ...key,
    catalog_service_slug: "admin-renamed-catalog",
    service_category: "llm",
  }));
  rerender(<Harness />);
  expect(
    within(screen.getByRole("tooltip")).getByText("Provider gateway routing is separate from this discovery order."),
  ).toBeVisible();
  expect(screen.queryByText("/api/v1/llm/admin-renamed-catalog")).toBeNull();
});
