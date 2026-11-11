import { MutationCache, QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, renderHook, waitFor } from "@testing-library/react";
import type { PropsWithChildren } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useAuthStore } from "@/stores/auth-store";
import type { User } from "@/types/api";
import { prepareMachineConsent, useMachineGrant, useMachineGrantList, useRevokeMachineGrant, useSaveMachineGrant } from "./use-machine-grants";
const transport = vi.fn<typeof fetch>();
const nodeId = "b7b0c3da-5d64-4b90-bbae-9bd7aa42104c";
const selection = { separation: "shared_legacy" as const, legacy_warning_acknowledged: true, assignments: { [nodeId]: { capabilities: { shell: true, files: true, browser: false, computer: false, developer_browser: false }, file_roots: ["/workspace"], service_ids: [] } } };
const grant = {
  id: "grant-one", source: { kind: "api_key", api_key_id: "key", api_key_state_version_at_approval: 3 },
  revision: 1, policy_revision: 1, status: "active", expires_at: "2099-01-01T00:00:00Z",
  created_at: "2026-01-01T00:00:00Z", updated_at: "2026-01-01T00:00:00Z", revoked_at: null, revoked_reason: null,
  assignments: { [nodeId]: { ...selection.assignments[nodeId], mode: "shared_legacy", revision: 1, saved_login_ids: [] } },
};
const json = (body: unknown, status = 200) => new Response(JSON.stringify(body), { status, headers: { "Content-Type": "application/json" } });
const auth = useAuthStore.getState();
function wrapper(client: QueryClient) { return ({ children }: PropsWithChildren) => <QueryClientProvider client={client}>{children}</QueryClientProvider>; }
beforeEach(() => { transport.mockReset(); vi.stubGlobal("fetch", transport); useAuthStore.setState({ user: { id: "owner-one" } as User }); });
afterEach(() => { vi.unstubAllGlobals(); useAuthStore.setState(auth); });
describe("Machine grant HTTP authority", () => {
  it.each([
    { kind: "api_key" as const, id: "other-key" },
    { kind: "oauth" as const, id: "oauth-client" },
  ])("rejects a source response that does not match $kind/$id", async (source) => {
    transport.mockResolvedValueOnce(json({ grant, source_and_grant_valid: true }));
    const client = new QueryClient();
    const { result, unmount } = renderHook(() => useMachineGrant(source), { wrapper: wrapper(client) });
    await waitFor(() => expect(result.current.isError).toBe(true));
    expect(result.current.data).toBeUndefined();
    expect(result.current.error?.message).toContain("different authentication source");
    expect(client.getQueryCache().getAll()[0]?.state.data).toBeUndefined();
    unmount(); client.clear();
  });
  it("rejects aggregate rows that do not include the requested Machine", async () => {
    transport.mockResolvedValueOnce(json({ external_grants: [grant] }));
    const client = new QueryClient();
    const { result, unmount } = renderHook(() => useMachineGrantList("other-machine"), { wrapper: wrapper(client) });
    await waitFor(() => expect(result.current.isError).toBe(true));
    expect(result.current.data).toBeUndefined();
    expect(result.current.error?.message).toContain("different Machine");
    expect(client.getQueryCache().getAll()[0]?.state.data).toBeUndefined();
    unmount(); client.clear();
  });
  it("reads the mounted aggregate without inferring live source validity", async () => {
    transport.mockResolvedValueOnce(json({ external_grants: [grant] }));
    const client = new QueryClient();
    const { result, unmount } = renderHook(() => useMachineGrantList(nodeId), { wrapper: wrapper(client) });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(transport.mock.calls[0]?.[0]).toBe(`/api/v1/remote-machines/${nodeId}/grants?limit=100`);
    expect(result.current.data?.external_grants[0]).toMatchObject({ id: "grant-one", assignments: grant.assignments });
    expect(result.current.data?.external_grants[0]).not.toHaveProperty("source_and_grant_valid");
    expect(transport).toHaveBeenCalledTimes(1);
    unmount(); client.clear();
  });
  it("requests bounded aggregate pages and follows the opaque composite cursor", async () => {
    const cursor = "2026-01-01T00:00:00.000Z|grant-one";
    transport
      .mockResolvedValueOnce(json({ external_grants: [grant], next_cursor: cursor }))
      .mockResolvedValueOnce(json({ external_grants: [], next_cursor: null }));
    const client = new QueryClient();
    const { result, unmount } = renderHook(() => useMachineGrantList(nodeId), { wrapper: wrapper(client) });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(transport.mock.calls[0]?.[0]).toBe(`/api/v1/remote-machines/${nodeId}/grants?limit=100`);
    await act(async () => { await result.current.fetchNextPage(); });
    expect(transport.mock.calls[1]?.[0]).toBe(`/api/v1/remote-machines/${nodeId}/grants?limit=100&cursor=${encodeURIComponent(cursor)}`);
    expect(result.current.data?.external_grants).toHaveLength(1);
    unmount(); client.clear();
  });
  it.each(["save", "revoke"] as const)("restarts discovery from the first page after a grant %s", async (operation) => {
    const cursor = "opaque-first-page";
    transport
      .mockResolvedValueOnce(json({ external_grants: [grant], next_cursor: cursor }))
      .mockResolvedValueOnce(json({ external_grants: [{ ...grant, id: "old-grant" }], next_cursor: null }))
      .mockResolvedValueOnce(json(grant))
      .mockResolvedValue(json({ external_grants: [grant], next_cursor: null }));
    const client = new QueryClient();
    const { result, unmount } = renderHook(() => ({ list: useMachineGrantList(nodeId), save: useSaveMachineGrant({ kind: "api_key", id: "key" }), revoke: useRevokeMachineGrant({ kind: "api_key", id: "key" }) }), { wrapper: wrapper(client) });
    await waitFor(() => expect(result.current.list.isSuccess).toBe(true));
    await act(async () => { await result.current.list.fetchNextPage(); });
    await waitFor(() => expect(result.current.list.data?.external_grants).toHaveLength(2));
    await act(async () => {
      if (operation === "save") await result.current.save.mutateAsync({ expected_grant_id: grant.id, expected_revision: 1, expected_state_version: 3, selection });
      else await result.current.revoke.mutateAsync({ expected_grant_id: grant.id, expected_revision: 1 });
    });
    await waitFor(() => expect(result.current.list.data?.external_grants).toHaveLength(1));
    expect(transport.mock.calls.slice(3).every(([url]) => !String(url).includes("cursor="))).toBe(true);
    unmount(); client.clear();
  });
  it.each([["api-keys", "personal"], ["consents", "me"]])("clears source checks and restarts grant pages after %j invalidation", async (...sourceKey) => {
    transport.mockImplementation(async (input) => {
      if (String(input).includes("/machine-grant")) return json({ grant, source_valid: true, source_validity_reason: null, grant_valid: true, grant_validity_reason: null, source_and_grant_valid: true });
      return json({ external_grants: [grant], next_cursor: String(input).includes("cursor=") ? null : "opaque" });
    });
    const client = new QueryClient();
    client.setQueryData(sourceKey, {});
    const { result, unmount } = renderHook(() => ({ list: useMachineGrantList(nodeId), live: useMachineGrant({ kind: "api_key", id: "key" }) }), { wrapper: wrapper(client) });
    await waitFor(() => expect(result.current.list.isSuccess && result.current.live.isSuccess).toBe(true));
    await act(async () => { await result.current.list.fetchNextPage(); });
    await waitFor(() => expect(result.current.list.data?.external_grants).toHaveLength(2));
    const before = transport.mock.calls.length;
    await act(async () => { await client.invalidateQueries({ queryKey: sourceKey }); });
    await waitFor(() => expect(result.current.list.data?.external_grants).toHaveLength(1));
    const laterPaths = transport.mock.calls.slice(before).map(([url]) => String(url));
    expect(laterPaths.some((path) => path.endsWith("/machine-grant"))).toBe(true);
    expect(laterPaths.filter((path) => path.includes("/grants?")).every((path) => !path.includes("cursor="))).toBe(true);
    unmount(); client.clear();
  });
  it("rejects a repeated aggregate cursor instead of looping", async () => {
    const cursor = "2026-01-01T00:00:00.000Z|grant-one";
    transport
      .mockResolvedValueOnce(json({ external_grants: [], next_cursor: cursor }))
      .mockResolvedValueOnce(json({ external_grants: [], next_cursor: cursor }));
    const client = new QueryClient();
    const { result, unmount } = renderHook(() => useMachineGrantList(nodeId), { wrapper: wrapper(client) });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    await act(async () => { await result.current.fetchNextPage().catch(() => undefined); });
    await waitFor(() => expect(result.current.isFetchNextPageError).toBe(true));
    expect(result.current.fetchNextPage).toBeDefined();
    unmount(); client.clear();
  });
  it("hides prior source validity after a failed live read", async () => {
    transport.mockResolvedValueOnce(json({ grant, source_and_grant_valid: true }));
    const client = new QueryClient();
    const { result, unmount } = renderHook(() => useMachineGrant({ kind: "api_key", id: "key" }), { wrapper: wrapper(client) });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    transport.mockResolvedValueOnce(json({ error: "unavailable", error_code: 503, message: "Unavailable" }, 503));
    await act(async () => { await result.current.refetch(); });
    await waitFor(() => expect(result.current.isError).toBe(true));
    expect(result.current.data).toBeUndefined();
    unmount(); client.clear();
  });
  it("hides prior aggregate grants after a failed live read", async () => {
    transport.mockResolvedValueOnce(json({ external_grants: [grant] }));
    const client = new QueryClient();
    const { result, unmount } = renderHook(() => useMachineGrantList(nodeId), { wrapper: wrapper(client) });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    transport.mockResolvedValueOnce(json({ error: "unavailable", error_code: 503, message: "Unavailable" }, 503));
    await act(async () => { await result.current.refetch(); });
    await waitFor(() => expect(result.current.isError).toBe(true));
    expect(result.current.data).toBeUndefined();
    unmount(); client.clear();
  });
  it("rejects a successful save response for a different source", async () => {
    transport.mockResolvedValueOnce(json({ ...grant, source: { ...grant.source, api_key_id: "other-key" } }));
    const client = new QueryClient();
    const { result, unmount } = renderHook(() => useSaveMachineGrant({ kind: "api_key", id: "key" }), { wrapper: wrapper(client) });
    await act(async () => { await expect(result.current.mutateAsync({ expected_grant_id: null, expected_revision: null, expected_state_version: 3, selection })).rejects.toThrow("different authentication source"); });
    expect(result.current.data).toBeUndefined();
    unmount(); client.clear();
  });
  it("rejects a successful revoke response for a different source", async () => {
    transport.mockResolvedValueOnce(json({ ...grant, status: "revoked", source: { ...grant.source, api_key_id: "other-key" } }));
    const client = new QueryClient();
    const { result, unmount } = renderHook(() => useRevokeMachineGrant({ kind: "api_key", id: "key" }), { wrapper: wrapper(client) });
    await act(async () => { await expect(result.current.mutateAsync({ expected_grant_id: grant.id, expected_revision: grant.revision })).rejects.toThrow("different authentication source"); });
    expect(result.current.data).toBeUndefined();
    unmount(); client.clear();
  });
  it.each(["source", "aggregate"] as const)("discards a late %s read after the account changes", async (kind) => {
    let resolve!: (response: Response) => void;
    transport.mockImplementationOnce(() => new Promise<Response>((done) => { resolve = done; }));
    transport.mockResolvedValueOnce(json(kind === "source" ? { grant: null, source_and_grant_valid: false } : { external_grants: [] }));
    const client = new QueryClient();
    const useRead: () => ReturnType<typeof useMachineGrant> | ReturnType<typeof useMachineGrantList> = kind === "source" ? () => useMachineGrant({ kind: "api_key", id: "key" }) : () => useMachineGrantList(nodeId);
    const { result, unmount } = renderHook(useRead, { wrapper: wrapper(client) });
    await waitFor(() => expect(transport).toHaveBeenCalledTimes(1));
    act(() => { useAuthStore.setState({ user: { id: "owner-two" } as User }); });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    await act(async () => { resolve(json(kind === "source" ? { grant, source_and_grant_valid: true } : { external_grants: [grant] })); });
    if (kind === "source") {
      await waitFor(() => expect(client.getQueryCache().findAll({ queryKey: ["machine-grants", "owner-one"] })[0]?.state.status).toBe("error"));
      expect(client.getQueryCache().findAll({ queryKey: ["machine-grants", "owner-one"] })[0]?.state.data).toBeUndefined();
    }
    expect(result.current.data).toMatchObject(kind === "source" ? { grant: null, source_and_grant_valid: false } : { external_grants: [] });
    unmount(); client.clear();
  });
  it("keeps safe grant metadata owner-scoped and treats 404 as an error", async () => {
    transport.mockResolvedValueOnce(json({ error: "not_found", error_code: 404, message: "Not found" }, 404));
    const client = new QueryClient();
    const { result, unmount } = renderHook(() => useMachineGrant({ kind: "api_key", id: "key" }), { wrapper: wrapper(client) });
    await waitFor(() => expect(result.current.isError).toBe(true));
    expect(result.current.data).toBeUndefined();
    expect(transport).toHaveBeenCalledTimes(1);
    expect(client.getQueryCache().getAll()[0]?.queryKey).toEqual(["machine-grants", "owner-one", "api_key", "key"]);
    unmount(); client.clear();
  });
  it("captures the reviewed owner before deferred mutation middleware", async () => {
    let resume!: () => void;
    const gate = new Promise<void>((resolve) => { resume = resolve; });
    const client = new QueryClient({ mutationCache: new MutationCache({ onMutate: () => gate }) });
    const { result, unmount } = renderHook(() => useSaveMachineGrant({ kind: "api_key", id: "key" }), { wrapper: wrapper(client) });
    let request!: Promise<unknown>;
    act(() => { request = result.current.mutateAsync({ expected_grant_id: null, expected_revision: null, expected_state_version: 3, selection }); });
    const rejected = expect(request).rejects.toThrow("account changed");
    act(() => { useAuthStore.setState({ user: { id: "owner-two" } as User }); });
    await act(async () => { resume(); await rejected; });
    expect(transport).not.toHaveBeenCalled();
    unmount(); client.clear();
  });
  it("keeps protected preparation out of Query caches and rejects late account-switched responses", async () => {
    let resolve!: (response: Response) => void;
    transport.mockImplementation(() => new Promise<Response>((done) => { resolve = done; }));
    const client = new QueryClient();
    const request = prepareMachineConsent("original-protected-request", selection);
    const rejected = expect(request).rejects.toThrow("account changed");
    await waitFor(() => expect(transport).toHaveBeenCalledTimes(1));
    useAuthStore.setState({ user: { id: "owner-two" } as User });
    resolve(json({ token: "owner-one-protected-token" }));
    await rejected;
    expect(client.getQueryCache().getAll()).toHaveLength(0);
    expect(client.getMutationCache().getAll()).toHaveLength(0);
    expect(transport.mock.calls[0]?.[1]?.headers).toMatchObject({ "Cache-Control": "no-store" });
    client.clear();
  });
});
