import {
  useMutation,
  useQuery,
  useInfiniteQuery,
  useQueryClient,
} from "@tanstack/react-query";
import { z } from "zod";
import { api, apiClient } from "@/lib/api-client";
import { useAuthStore } from "@/stores/auth-store";
import {
  adapterBindingSchema,
  adapterDescriptorSchema,
  adapterAccountSchema,
  adapterAllocationSchema,
  type AdapterAccountInput,
  type AdapterAllocationInput,
  type AdapterSetup,
  type AdapterBinding,
} from "@/schemas/channel-adapters";

function guard(actor: string | undefined) {
  if (!actor || actor !== useAuthStore.getState().user?.id)
    throw new Error("Account changed. Reopen NyxBot settings.");
}
export function useChannelAdapterCatalog() {
  const actor = useAuthStore((s) => s.user?.id);
  return useQuery({
    queryKey: ["channel-adapters", actor],
    enabled: !!actor,
    queryFn: async () => {
      guard(actor);
      return z.object({ adapters: z.array(adapterDescriptorSchema) }).parse(
        await api.get("/channel-adapters", {
          authorityGuard: () => guard(actor),
        }),
      ).adapters;
    },
  });
}
export function useChannelAdapterBindings() {
  const actor = useAuthStore((s) => s.user?.id);
  const query = useInfiniteQuery({
    queryKey: ["channel-adapter-bindings", actor],
    enabled: !!actor,
    gcTime: 0,
    retry: false,
    initialPageParam: "",
    getNextPageParam: (page: {
      bindings: AdapterBinding[];
      next_after_id?: string | null;
    }) => page.next_after_id ?? undefined,
    queryFn: async ({ pageParam }) => {
      guard(actor);
      return z
        .object({
          bindings: z.array(adapterBindingSchema),
          next_after_id: z.string().nullish(),
        })
        .parse(
          await api.get(
            `/channel-adapter-bindings${pageParam ? `?after=${encodeURIComponent(pageParam)}` : ""}`,
            { authorityGuard: () => guard(actor) },
          ),
        );
    },
  });
  return { ...query, data: query.data?.pages.flatMap((p) => p.bindings) };
}
export function useChannelAdapterActions() {
  const actor = useAuthStore((s) => s.user?.id);
  const client = useQueryClient();
  const refresh = () =>
    client.invalidateQueries({ queryKey: ["channel-adapter-bindings", actor] });
  const create = useMutation({
    retry: false,
    mutationFn: async (input: AdapterSetup) => {
      guard(actor);
      return adapterBindingSchema.parse(
        await apiClient("/channel-adapter-bindings", {
          method: "POST",
          body: input,
          authorityGuard: () => guard(actor),
        }),
      );
    },
    onSuccess: refresh,
  });
  const disable = useMutation({
    retry: false,
    mutationFn: async (id: string) => {
      guard(actor);
      await apiClient(`/channel-adapter-bindings/${id}/disable`, {
        method: "POST",
        authorityGuard: () => guard(actor),
      });
    },
    onSuccess: refresh,
  });
  const remove = useMutation({
    retry: false,
    mutationFn: async (id: string) => {
      guard(actor);
      await apiClient(`/channel-adapter-bindings/${id}`, {
        method: "DELETE",
        authorityGuard: () => guard(actor),
      });
    },
    onSuccess: refresh,
  });
  const verify = useMutation({
    retry: false,
    mutationFn: async (id: string) => {
      guard(actor);
      return adapterBindingSchema.parse(
        await apiClient(`/channel-adapter-bindings/${id}/verify`, {
          method: "POST",
          body: {},
          authorityGuard: () => guard(actor),
        }),
      );
    },
    onSuccess: refresh,
  });
  // Secret deliveries are mutations, never persisted query cache entries.
  const customSetup = useMutation({
    retry: false,
    gcTime: 0,
    mutationFn: async (id: string) => {
      guard(actor);
      return z
        .object({
          url: z.url(),
          api_key: z.string(),
          model: z.string(),
          correlation: z.string(),
        })
        .parse(
          await apiClient(`/channel-adapter-bindings/${id}/custom-llm-setup`, {
            method: "POST",
            body: {},
            authorityGuard: () => guard(actor),
          }),
        );
    },
  });
  const customSession = useMutation({
    retry: false,
    gcTime: 0,
    mutationFn: async (id: string) => {
      guard(actor);
      return z
        .object({
          session_id: z.uuid(),
          expires_at: z.string(),
          extra_body: z.record(z.string(), z.string()),
        })
        .parse(
          await apiClient(`/channel-adapter-bindings/${id}/sessions`, {
            method: "POST",
            body: {},
            authorityGuard: () => guard(actor),
          }),
        );
    },
  });
  const endSession = useMutation({
    retry: false,
    mutationFn: async ({
      id,
      sessionId,
    }: {
      id: string;
      sessionId: string;
    }) => {
      guard(actor);
      await apiClient(
        `/channel-adapter-bindings/${id}/sessions/${sessionId}/end`,
        {
          method: "POST",
          authorityGuard: () => guard(actor),
        },
      );
    },
  });
  return {
    create,
    disable,
    remove,
    verify,
    customSetup,
    customSession,
    endSession,
  };
}

export function useAdminAdapterResources() {
  const actor = useAuthStore((s) => s.user?.id);
  const client = useQueryClient();
  const accountPages = useInfiniteQuery({
    queryKey: ["admin", "channel-adapter-accounts", actor],
    retry: false,
    initialPageParam: "",
    getNextPageParam: (page: {
      accounts: z.infer<typeof adapterAccountSchema>[];
      next_after_id?: string | null;
    }) => page.next_after_id ?? undefined,
    queryFn: async ({ pageParam }) =>
      z
        .object({
          accounts: z.array(adapterAccountSchema),
          next_after_id: z.string().nullish(),
        })
        .parse(
          await api.get(
            `/admin/channel-adapter-accounts${pageParam ? `?after=${encodeURIComponent(pageParam)}` : ""}`,
            { authorityGuard: () => guard(actor) },
          ),
        ),
  });
  const allocationPages = useInfiniteQuery({
    queryKey: ["admin", "channel-adapter-allocations", actor],
    retry: false,
    initialPageParam: "",
    getNextPageParam: (page: {
      allocations: z.infer<typeof adapterAllocationSchema>[];
      next_after_id?: string | null;
    }) => page.next_after_id ?? undefined,
    queryFn: async ({ pageParam }) =>
      z
        .object({
          allocations: z.array(adapterAllocationSchema),
          next_after_id: z.string().nullish(),
        })
        .parse(
          await api.get(
            `/admin/channel-adapter-allocations${pageParam ? `?after=${encodeURIComponent(pageParam)}` : ""}`,
            { authorityGuard: () => guard(actor) },
          ),
        ),
  });
  const accounts = {
    ...accountPages,
    data: accountPages.data?.pages.flatMap((p) => p.accounts),
  };
  const allocations = {
    ...allocationPages,
    data: allocationPages.data?.pages.flatMap((p) => p.allocations),
  };
  const refresh = async () => {
    await Promise.all([
      client.invalidateQueries({
        queryKey: ["admin", "channel-adapter-accounts", actor],
      }),
      client.invalidateQueries({
        queryKey: ["admin", "channel-adapter-allocations", actor],
      }),
    ]);
  };
  const configure = useMutation({
    retry: false,
    mutationFn: async (body: AdapterAccountInput) =>
      apiClient("/admin/channel-adapter-accounts", {
        method: "POST",
        body,
        authorityGuard: () => guard(actor),
      }),
    onSuccess: refresh,
  });
  const allocate = useMutation({
    retry: false,
    mutationFn: async (body: AdapterAllocationInput) =>
      apiClient("/admin/channel-adapter-allocations", {
        method: "POST",
        body,
        authorityGuard: () => guard(actor),
      }),
    onSuccess: refresh,
  });
  const revoke = useMutation({
    retry: false,
    mutationFn: async (id: string) =>
      apiClient(`/admin/channel-adapter-allocations/${id}`, {
        method: "DELETE",
        authorityGuard: () => guard(actor),
      }),
    onSuccess: refresh,
  });
  const removeAccount = useMutation({
    retry: false,
    mutationFn: async (id: string) =>
      apiClient(`/admin/channel-adapter-accounts/${id}`, {
        method: "DELETE",
        authorityGuard: () => guard(actor),
      }),
    onSuccess: refresh,
  });
  return { accounts, allocations, configure, allocate, revoke, removeAccount };
}
