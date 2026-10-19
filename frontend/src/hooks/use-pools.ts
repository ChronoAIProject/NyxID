import {
  hashKey,
  useInfiniteQuery,
  useMutation,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import { useEffect, useRef } from "react";
import { api, apiClient, ApiError } from "@/lib/api-client";
import type {
  PoolCandidatesResponse,
  CreateServicePoolInput,
  ServicePool,
  ServicePoolListResponse,
  ServicePoolMember,
  SetPoolMembersInput,
  UpdateServicePoolInput,
} from "@/schemas/pools";

const SERVICE_POOLS_KEY = ["service-pools"] as const;

export function useServicePools(orgId?: string) {
  return useQuery({
    queryKey: [...SERVICE_POOLS_KEY, "list", orgId],
    queryFn: async (): Promise<readonly ServicePool[]> => {
      const res = await api.get<ServicePoolListResponse>(
        orgId
          ? `/service-pools?org_id=${encodeURIComponent(orgId)}`
          : "/service-pools",
      );
      return res.pools;
    },
  });
}

export function useServicePool(poolId: string | null | undefined) {
  return useQuery({
    queryKey: [...SERVICE_POOLS_KEY, poolId],
    queryFn: async (): Promise<ServicePool> => {
      return api.get<ServicePool>(
        `/service-pools/${encodeURIComponent(poolId!)}`,
      );
    },
    enabled: Boolean(poolId),
  });
}

function invalidatePools(
  queryClient: ReturnType<typeof useQueryClient>,
  poolId?: string,
) {
  void queryClient.invalidateQueries({ queryKey: SERVICE_POOLS_KEY });
  void queryClient.invalidateQueries({ queryKey: ["keys"] });
  if (poolId) {
    void queryClient.invalidateQueries({
      queryKey: [...SERVICE_POOLS_KEY, poolId],
    });
  }
}

export function useCreateServicePool() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (input: CreateServicePoolInput): Promise<ServicePool> => {
      return api.post<ServicePool>("/service-pools", input);
    },
    onSuccess: () => invalidatePools(queryClient),
  });
}

export function useUpdateServicePool() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (
      input: UpdateServicePoolInput & { readonly poolId: string },
    ): Promise<ServicePool> => {
      const { poolId, ...body } = input;
      return api.put<ServicePool>(
        `/service-pools/${encodeURIComponent(poolId)}`,
        body,
      );
    },
    onSuccess: (_data, variables) =>
      invalidatePools(queryClient, variables.poolId),
    onError: (error, variables) => {
      if (error instanceof ApiError && error.status === 409)
        invalidatePools(queryClient, variables.poolId);
    },
  });
}

export function useReloadServicePool() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: (poolId: string) =>
      queryClient.fetchQuery({
        queryKey: [...SERVICE_POOLS_KEY, poolId],
        staleTime: 0,
        queryFn: () =>
          api.get<ServicePool>(`/service-pools/${encodeURIComponent(poolId)}`),
      }),
  });
}

export function useDeleteServicePool() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (poolId: string): Promise<void> => {
      return api.delete<void>(`/service-pools/${encodeURIComponent(poolId)}`);
    },
    onSuccess: () => invalidatePools(queryClient),
  });
}

export function useSetServicePoolMembers() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (
      input: SetPoolMembersInput & { readonly poolId: string },
    ): Promise<ServicePool> => {
      const { poolId, members } = input;
      return api.put<ServicePool>(
        `/service-pools/${encodeURIComponent(poolId)}/members`,
        { members },
      );
    },
    onSuccess: (_data, variables) =>
      invalidatePools(queryClient, variables.poolId),
  });
}

export function useAddServicePoolMember() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (
      input: ServicePoolMember & { readonly poolId: string },
    ): Promise<ServicePool> => {
      const { poolId, ...member } = input;
      return api.post<ServicePool>(
        `/service-pools/${encodeURIComponent(poolId)}/members`,
        member,
      );
    },
    onSuccess: (_data, variables) =>
      invalidatePools(queryClient, variables.poolId),
  });
}

export function useRemoveServicePoolMember() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (input: {
      readonly poolId: string;
      readonly userServiceId: string;
    }): Promise<ServicePool> => {
      return api.delete<ServicePool>(
        `/service-pools/${encodeURIComponent(input.poolId)}/members/${encodeURIComponent(input.userServiceId)}`,
      );
    },
    onSuccess: (_data, variables) =>
      invalidatePools(queryClient, variables.poolId),
  });
}

export interface PoolInspectionOptions {
  poolId?: string;
  orgId?: string;
  contract?: "same_api" | "ai_chat";
  checkOperation?: boolean;
  method?: string;
  path?: string;
  search?: string;
  peerIds?: string[];
  selectedOnly?: boolean;
  declaredPeerIds?: string[];
  strategy?: "priority" | "round_robin" | "weighted";
}
function inspectionPath(
  options: PoolInspectionOptions,
  health: boolean,
  after?: string,
) {
  const base = options.poolId
    ? `/service-pools/${encodeURIComponent(options.poolId)}/${health ? "health" : "candidates"}`
    : "/service-pools/candidates";
  const query = new URLSearchParams({
    member_contract: options.contract ?? "same_api",
    limit: "100",
  });
  if (options.checkOperation != null)
    query.set("check_operation", String(options.checkOperation));
  if (options.method) query.set("method", options.method);
  if (options.path != null) query.set("path", options.path);
  if (options.orgId) query.set("org_id", options.orgId);
  if (options.strategy) query.set("strategy", options.strategy);
  if (options.declaredPeerIds)
    query.set("declared_peer_ids", options.declaredPeerIds.join(","));
  if (options.selectedOnly) query.set("selected_only", "true");
  if (options.peerIds) query.set("peer_ids", options.peerIds.join(","));
  if (options.search) query.set("search", options.search);
  if (after) query.set("after", after);
  return `${base}?${query.toString()}`;
}
export function usePoolCandidates(
  options: PoolInspectionOptions,
  enabled = true,
) {
  const identity = options.selectedOnly
    ? options
    : {
        ...options,
        peerIds: undefined,
        declaredPeerIds: undefined,
      };
  const queryKey = [...SERVICE_POOLS_KEY, "candidates", identity];
  const scope = hashKey(queryKey);
  const peers = JSON.stringify([options.peerIds, options.declaredPeerIds]);
  const previous = useRef({ scope, peers });
  const result = useInfiniteQuery({
    queryKey,
    enabled,
    // Cached searches may have been checked against a different draft selection.
    staleTime: 0,
    initialPageParam: undefined as string | undefined,
    queryFn: async ({ pageParam, signal }) => ({
      ...(await apiClient<PoolCandidatesResponse>(
        inspectionPath(options, false, pageParam),
        { signal },
      )),
      inspectionPeers: peers,
    }),
    getNextPageParam: (page) =>
      page.has_more ? (page.next_cursor ?? undefined) : undefined,
  });
  const { refetch, isFetching, isError } = result;
  const isCheckingCompatibility = Boolean(
    result.data?.pages.some((page) => page.inspectionPeers !== peers),
  );
  useEffect(() => {
    const changed =
      previous.current.scope === scope && previous.current.peers !== peers;
    previous.current = { scope, peers };
    // Finish pending pagination before refreshing every loaded page for the draft.
    if (
      enabled &&
      !options.selectedOnly &&
      !isFetching &&
      isCheckingCompatibility &&
      (changed || !isError)
    ) {
      void refetch({ cancelRefetch: false });
    }
  }, [
    scope,
    peers,
    enabled,
    options.selectedOnly,
    refetch,
    isFetching,
    isError,
    isCheckingCompatibility,
  ]);
  return { ...result, isCheckingCompatibility };
}
export function usePoolHealth(options: PoolInspectionOptions) {
  return useQuery({
    queryKey: [...SERVICE_POOLS_KEY, "health", options],
    enabled: Boolean(options.poolId),
    queryFn: () =>
      api.get<PoolCandidatesResponse>(inspectionPath(options, true)),
    refetchInterval: 15000,
  });
}
export function useResetPoolHealth() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: ({
      poolId,
      userServiceId,
    }: {
      poolId: string;
      userServiceId?: string;
    }) =>
      api.post(`/service-pools/${encodeURIComponent(poolId)}/health/reset`, {
        user_service_id: userServiceId ?? null,
      }),
    onSuccess: () => invalidatePools(client),
  });
}
