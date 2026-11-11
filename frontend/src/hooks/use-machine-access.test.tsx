import { QueryClient, QueryClientProvider, focusManager } from "@tanstack/react-query";
import { act, renderHook } from "@testing-library/react";
import type { PropsWithChildren } from "react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { useAuthStore } from "@/stores/auth-store";
import type { User } from "@/types/api";
import { useMachineAccess, useRequestMachineContext } from "./use-machine-access";
const { read } = vi.hoisted(() => ({ read: vi.fn() }));
vi.mock("@/lib/assistant/assistant-http", () => ({ assistantJson: read }));
const off = { shell: false, files: false, browser: false, computer: false, developer_browser: false };
const machine = { node_id: "node", name: "Machine", revision: 3, protocol_v2: true, can_edit: true, capabilities: off, ceiling: off, legacy: false, saved_login_ids: null, revocation_pending: true, delivery_pending: true };
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

it("parses actionable missing selection alongside old nodes without capability reports", async () => {
  read.mockResolvedValue([{ ...machine, mode:"selection_required", selection_required:true, selection_reason:"Choose a supported mode or explicitly approve legacy.", separation_choice:null, separated:null }]);
  const {result,unmount}=setup();
  await advance(10);
  expect(result.current.data?.[0]).toMatchObject({ mode:"selection_required", selection_required:true, separation_choice:null });
  unmount();
});
it("binds a legacy owner-card request to the account that reviewed the warning", async () => {
  const wrapper = ({children}:PropsWithChildren) => <QueryClientProvider client={client}>{children}</QueryClientProvider>;
  const {result,unmount}=renderHook(() => useRequestMachineContext("agent"),{wrapper});
  const reviewed=result.current.mutateAsync;
  act(() => useAuthStore.setState({user:{id:"other-owner"} as User}));
  await expect(reviewed({node:"node",selection:{mode:"shared_legacy",expected_revision:3,capabilities:off,saved_login_ids:[]}})).rejects.toThrow(/account changed/i);
  expect(read).not.toHaveBeenCalled();
  unmount();
});
