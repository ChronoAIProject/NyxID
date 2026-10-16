import { useInfiniteQuery, useMutation, useQuery, useQueryClient, type MutateOptions, type UseMutationOptions } from "@tanstack/react-query";
import { useCallback, useEffect } from "react";
import { api, apiClient, ApiError } from "@/lib/api-client";
import { useAuthStore } from "@/stores/auth-store";
import { machineGrantListSchema, machineGrantReadSchema, machineGrantSchema, machineGrantSelectionSchema, type MachineGrantSelection, type MachineGrantSource } from "@/schemas/machine-grants";

export function machineGrantPath(source: MachineGrantSource) {
  return source.kind === "api_key"
    ? `/api-keys/${encodeURIComponent(source.id)}/machine-grant`
    : `/users/me/consents/${encodeURIComponent(source.id)}/machine-grant`;
}
export function machineGrantUnavailable(error: unknown): boolean {
  return error instanceof ApiError && [404, 405, 501].includes(error.status);
}
function assertOwner(owner: string | undefined): asserts owner is string {
  if (!owner || useAuthStore.getState().user?.id !== owner) throw new Error("Your account changed. Reload before managing Machine permissions.");
}
function authority(owner: string | undefined) { return { authorityGuard: () => assertOwner(owner) }; }
function readGrantResponse(value: unknown, path: string) {
  const grant = machineGrantSchema.parse(value);
  const source: MachineGrantSource = grant.source.kind === "api_key"
    ? { kind: "api_key", id: grant.source.api_key_id }
    : { kind: "oauth", id: grant.source.oauth_client_id };
  if (machineGrantPath(source) !== path) throw new Error("The server returned Machine permissions for a different authentication source.");
  return grant;
}

/** Capture both the reviewed source and owner before async mutation middleware. */
function useGrantOwnerMutation<Data, Body>(source: MachineGrantSource, options: UseMutationOptions<Data, Error, { owner: string; path: string; body: Body }>) {
  const owner = useAuthStore((state) => state.user?.id);
  const path = machineGrantPath(source);
  const mutation = useMutation(options);
  const dispatch = mutation.mutate;
  const dispatchAsync = mutation.mutateAsync;
  const mutate = useCallback((body: Body, callbacks?: MutateOptions<Data, Error, { owner: string; path: string; body: Body }>) => {
    assertOwner(owner);
    dispatch({ owner, path, body }, callbacks);
  }, [dispatch, owner, path]);
  const mutateAsync = useCallback(async (body: Body, callbacks?: MutateOptions<Data, Error, { owner: string; path: string; body: Body }>) => {
    assertOwner(owner);
    return dispatchAsync({ owner, path, body }, callbacks);
  }, [dispatchAsync, owner, path]);
  return { ...mutation, mutate, mutateAsync };
}
/** Source management already invalidates these caches. Reset only this mounted
 * grant observer so source changes discard old live checks and pagination cursors. */
function useSourceInvalidationReset(owner: string | undefined, kind: string, id: string, enabled = true) {
  const client = useQueryClient();
  useEffect(() => {
    if (!owner || !enabled) return;
    return client.getQueryCache().subscribe((event) => {
      if (event.type !== "updated" || event.action.type !== "invalidate") return;
      const [domain, scope] = event.query.queryKey;
      if (domain !== "api-keys" && !(domain === "consents" && scope === "me")) return;
      if (useAuthStore.getState().user?.id !== owner) return;
      void client.resetQueries({ queryKey: ["machine-grants", owner, kind, id], exact: true });
    });
  }, [client, owner, kind, id, enabled]);
}

export function useMachineGrant(source: MachineGrantSource, enabled = true) {
  const owner = useAuthStore((state) => state.user?.id);
  const path = machineGrantPath(source);
  useSourceInvalidationReset(owner, source.kind, source.id, enabled);
  const query = useQuery({
    queryKey: ["machine-grants", owner, source.kind, source.id],
    queryFn: async () => {
      const result = machineGrantReadSchema.parse(await api.get(path, authority(owner)));
      if (result.grant) readGrantResponse(result.grant, path);
      return result;
    },
    enabled: enabled && Boolean(owner && source.id),
    retry: false,
    refetchInterval: (query) => query.state.status === "error" ? false : 15000,
  });
  return { ...query, data: query.isError ? undefined : query.data };
}
type SaveBody = { expected_revision: number | null; expected_grant_id: string | null; expected_state_version?: number; selection: MachineGrantSelection };
export function useSaveMachineGrant(source: MachineGrantSource) {
  const query = useQueryClient();
  return useGrantOwnerMutation(source, {
    mutationFn: async ({ owner, path, body }: { owner: string; path: string; body: SaveBody }) => {
      const request = { ...body, selection: machineGrantSelectionSchema.parse(body.selection) };
      return readGrantResponse(await apiClient(path, { method: "PUT", body: request, ...authority(owner) }), path);
    },
    retry: false,
    onSuccess: async (_data, { owner }) => {
      if (useAuthStore.getState().user?.id !== owner) return;
      await Promise.all([
        query.resetQueries({ queryKey: ["machine-grants", owner] }),
        query.invalidateQueries({ queryKey: ["api-keys"] }),
      ]);
    },
    onError: (error, { owner }) => {
      if (useAuthStore.getState().user?.id === owner && error instanceof ApiError && error.status === 409) void query.invalidateQueries({ queryKey: ["api-keys"] });
    },
  });
}
export function useRevokeMachineGrant(source: MachineGrantSource) {
  const query = useQueryClient();
  return useGrantOwnerMutation(source, {
    mutationFn: async ({ owner, path, body }: { owner: string; path: string; body: { expected_revision: number; expected_grant_id: string } }) => readGrantResponse(await apiClient(path, { method: "DELETE", body, ...authority(owner) }), path),
    retry: false,
    onSuccess: (_data, { owner }) => {
      if (useAuthStore.getState().user?.id === owner) return query.resetQueries({ queryKey: ["machine-grants", owner] });
    },
  });
}
/** The one-time signed approval is kept only by its page, never in Query caches. */
export async function prepareMachineConsent(consentRequest: string, selection: MachineGrantSelection): Promise<string> {
  const owner = useAuthStore.getState().user?.id;
  assertOwner(owner);
  const result = await apiClient<{ token: string }>("/users/me/oauth-machine-consent-requests", {
    method: "POST",
    body: { consent_request: consentRequest, selection: machineGrantSelectionSchema.parse(selection) },
    headers: { "Cache-Control": "no-store", Pragma: "no-cache" },
    ...authority(owner),
  });
  if (typeof result.token !== "string" || !result.token) throw new Error("The server did not return a protected Machine selection");
  return result.token;
}

/** Existing protected OAuth request handle read without persisting the signed token. */
export async function readProtectedConsentRequest(handle: string, signal?: AbortSignal): Promise<string> {
  const owner = useAuthStore.getState().user?.id;
  assertOwner(owner);
  const response = await apiClient<{ token: string }>(`/users/me/oauth-consent-requests/${encodeURIComponent(handle)}`, {
    signal, headers: { "Cache-Control": "no-store", Pragma: "no-cache" }, ...authority(owner),
  });
  if (typeof response.token !== "string" || !response.token) throw new Error("The protected authorization request is unavailable");
  return response.token;
}

export function useMachineGrantList(machineId: string) {
  const owner = useAuthStore((state) => state.user?.id);
  useSourceInvalidationReset(owner, "machine", machineId);
  const query = useInfiniteQuery({
    queryKey: ["machine-grants", owner, "machine", machineId],
    initialPageParam: null as string | null,
    queryFn: async ({ pageParam, signal }) => {
      const params = new URLSearchParams({ limit: "100" });
      if (pageParam) params.set("cursor", pageParam);
      const result = machineGrantListSchema.parse(await apiClient(`/remote-machines/${encodeURIComponent(machineId)}/grants?${params}`, { ...authority(owner), signal }));
      if (result.external_grants.some((grant) => !Object.hasOwn(grant.assignments, machineId))) throw new Error("The server returned an external grant for a different Machine.");
      if (result.next_cursor !== null && result.next_cursor === pageParam) throw new Error("The server returned an invalid cursor.");
      return result;
    },
    getNextPageParam: (page) => page.next_cursor ?? undefined,
    enabled: Boolean(owner && machineId),
    retry: false,
    staleTime: 0,
    gcTime: 0,
    refetchOnMount: "always",
    refetchOnWindowFocus: "always",
    refetchInterval: (query) => query.state.status === "error" ? false : 15000,
  });
  const data = query.isError || !query.data ? undefined : {
    external_grants: query.data.pages.flatMap((page) => page.external_grants),
    next_cursor: query.data.pages.at(-1)?.next_cursor,
  };
  return { ...query, data };
}
