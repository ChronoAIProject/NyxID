import { QueryClient, QueryClientProvider, focusManager } from "@tanstack/react-query";
import { act, renderHook } from "@testing-library/react";
import type { PropsWithChildren } from "react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { machineAccessFixture } from "@/test/machine-access-fixture";
import { useAuthStore } from "@/stores/auth-store";
import type { User } from "@/types/api";
import { useMachineAccess, usePrepareMachineContext } from "./use-machine-access";
const { read } = vi.hoisted(() => ({ read: vi.fn() }));
vi.mock("@/lib/assistant/assistant-http", () => ({ assistantJson: read }));
const off = { shell: false, files: false, browser: false, computer: false, developer_browser: false };
const machine = { ...machineAccessFixture, node_id: "node", name: "Machine", revision: 3, protocol_v2: true, can_edit: true, capabilities: off, ceiling: off, legacy: false, saved_login_ids: null, revocation_pending: true, delivery_pending: true };
let client: QueryClient;
beforeEach(() => {
  vi.useFakeTimers();
  focusManager.setFocused(true);
  read.mockReset().mockResolvedValue([machine]);
  useAuthStore.setState({ user: { id: "owner" } as User });
  client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
});
afterEach(() => { client.clear(); focusManager.setFocused(undefined); vi.useRealTimers(); });
function setup() {
  const wrapper = ({ children }: PropsWithChildren) => <QueryClientProvider client={client}>{children}</QueryClientProvider>;
  return renderHook(({ enabled }) => useMachineAccess("agent", enabled), { wrapper, initialProps: { enabled: true } });
}
async function advance(ms: number) { await act(async () => { await vi.advanceTimersByTimeAsync(ms); }); }
it("refreshes pending status, stops on acknowledgement, and supports manual refresh", async () => {
  const { result, unmount } = setup();
  await advance(10);
  expect(read).toHaveBeenCalledTimes(1);
  read.mockResolvedValue([{ ...machine, revocation_pending: false, delivery_pending: false }]);
  await advance(3_000);
  expect(result.current.data?.[0]?.delivery_pending).toBe(false);
  expect(read).toHaveBeenCalledTimes(2);
  await advance(60_000);
  expect(read).toHaveBeenCalledTimes(2);
  await act(async () => { await result.current.refetch(); });
  expect(read).toHaveBeenCalledTimes(3);
  unmount();
});
it("bounds an offline delivery to one minute and stops polling when inactive", async () => {
  const { result, rerender, unmount } = setup();
  await advance(65_000);
  const calls = read.mock.calls.length;
  expect(calls).toBeGreaterThan(1);
  expect(calls).toBeLessThanOrEqual(21);
  await advance(60_000);
  expect(read).toHaveBeenCalledTimes(calls);
  await act(async () => { await result.current.refetch(); });
  expect(read).toHaveBeenCalledTimes(calls + 1);
  rerender({ enabled: false });
  await advance(60_000);
  expect(read).toHaveBeenCalledTimes(calls + 1);
  unmount();
});
it("does not poll a hidden page or a settled machine", async () => {
  const { unmount } = setup();
  await advance(10);
  focusManager.setFocused(false);
  await advance(12_000);
  expect(read).toHaveBeenCalledTimes(1);
  unmount();
  read.mockResolvedValue([{ ...machine, revocation_pending: false, delivery_pending: false }]);
  client.clear();
  focusManager.setFocused(true);
  const next = setup();
  await advance(10);
  const calls = read.mock.calls.length;
  await advance(60_000);
  expect(read).toHaveBeenCalledTimes(calls);
  next.unmount();
});

it("observes cleanup-only pending status without treating settled execution or delivery as cleanup evidence", async () => {
  read.mockResolvedValue([{ ...machine, revocation_pending: false, delivery_pending: false, quarantine_pending: true }]);
  const { result, unmount } = setup();
  await advance(10);
  expect(read).toHaveBeenCalledTimes(1);
  await advance(3_000);
  expect(read).toHaveBeenCalledTimes(2);
  expect(result.current.data?.[0]?.quarantine_pending).toBe(true);
  read.mockResolvedValue([{ ...machine, revocation_pending: false, delivery_pending: false, quarantine_pending: false }]);
  await advance(3_000);
  expect(result.current.data?.[0]?.quarantine_pending).toBe(false);
  const calls = read.mock.calls.length;
  await advance(60_000);
  expect(read).toHaveBeenCalledTimes(calls);
  unmount();
});

it("continues bounded observation for hosted configuration after Agent authority is applied", async () => {
  read.mockResolvedValue([{ ...machine, revocation_pending: false, delivery_pending: false, authorization_applied: true,
    machine_configuration: { version: 2, revision: 3, digest: "a".repeat(64), runtime_generation: 4, state: "pending", reason: "current_runtime_configuration_evidence_required" }, environment_applied: false }]);
  const { result, unmount } = setup();
  await advance(3_100);
  expect(read).toHaveBeenCalledTimes(2);
  expect(result.current.data?.[0]?.authorization_applied).toBe(true);
  expect(result.current.data?.[0]?.environment_applied).toBe(false);
  await advance(60_000);
  const calls = read.mock.calls.length;
  await advance(60_000);
  expect(read).toHaveBeenCalledTimes(calls);
  unmount();
});

it("refuses an access response without the current owner-review policy version", async () => {
  const { context_policy_version: omitted, ...incomplete } = { ...machine, context_policy_version: "machine-context-mode-v1" };
  void omitted;
  read.mockResolvedValue([incomplete]);
  const { result, unmount } = setup();
  await advance(10);
  expect(result.current.isError).toBe(true);
  expect(result.current.data).toBeUndefined();
  unmount();
});

it("parses actionable missing selection alongside old nodes without capability reports", async () => {
  read.mockResolvedValue([{ ...machine, mode:"selection_required", selection_required:true, selection_reason:"Choose a supported mode or explicitly approve legacy.", separation_choice:null, separated:null }]);
  const {result,unmount}=setup();
  await advance(10);
  expect(result.current.data?.[0]).toMatchObject({ mode:"selection_required", selection_required:true, separation_choice:null });
  unmount();
});
it("binds a legacy owner-card request to the account that reviewed the warning", async () => {
  const wrapper = ({children}:PropsWithChildren) => <QueryClientProvider client={client}>{children}</QueryClientProvider>;
  const {result,unmount}=renderHook(() => usePrepareMachineContext("agent"),{wrapper});
  const reviewed=result.current.mutateAsync;
  act(() => useAuthStore.setState({user:{id:"other-owner"} as User}));
  await expect(reviewed({node:"node",body:{mode:"shared_legacy",policy_version:"machine-context-mode-v1",acknowledge_separation:true}})).rejects.toThrow(/account changed/i);
  expect(read).not.toHaveBeenCalled();
  unmount();
});
