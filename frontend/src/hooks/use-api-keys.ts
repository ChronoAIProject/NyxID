import { useMemo } from "react";
import { useMutation, useQueries, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import type {
  ApiKey,
  ApiKeyCreateResponse,
  ApiKeyUsage,
  ApiKeyUsageListResponse,
} from "@/types/api";
import type { CreateApiKeyFormData } from "@/schemas/api-keys";
import { useOrgs } from "./use-orgs";

interface UseApiKeysParams {
  /** `null` or omitted lists the caller's personal keys. When set, lists
   *  keys owned by the given org (admin-only). */
  readonly orgId?: string | null;
}

/**
 * List NyxID API keys. Defaults to the caller's personal keys. Pass an
 * `orgId` to list keys owned by a specific org (caller must be an admin).
 *
 * See `useAllAdminedApiKeys` for the aggregated view that the Agent Keys
 * table uses.
 */
export function useApiKeys(params: UseApiKeysParams = {}) {
  const orgId = params.orgId ?? null;
  return useQuery({
    queryKey: ["api-keys", orgId ?? "personal"],
    queryFn: async (): Promise<readonly ApiKey[]> => {
      const path = orgId
        ? `/api-keys?org_id=${encodeURIComponent(orgId)}`
        : "/api-keys";
      const res = await api.get<{ readonly keys: readonly ApiKey[] }>(path);
      return res.keys;
    },
  });
}

/**
 * Aggregate personal + every admined-org API key list into a single array.
 *
 * The backend treats `/api-keys` and `/api-keys?org_id=X` as separate scopes
 * (each requires a different ownership check). The Agent Keys table needs a
 * single grid that lets org admins manage both kinds in one place, so this
 * hook fires one request per scope in parallel and flattens the results.
 *
 * `isLoading` is true until the first page (personal keys) resolves — at
 * that point the table can render, and org-scope queries fill in as they
 * complete. Individual org-scope errors are swallowed (logged) so a single
 * failed org does not hide the rest of the keys.
 */
export function useAllAdminedApiKeys() {
  const personal = useApiKeys();
  const { data: orgs } = useOrgs();

  const adminOrgIds = useMemo(
    () => (orgs ?? []).filter((o) => o.your_role === "admin").map((o) => o.id),
    [orgs],
  );

  const orgQueries = useQueries({
    // Share the cache with direct `useApiKeys({ orgId })` callers (e.g.
    // channel-bot-detail) by using the same `["api-keys", orgId]` key.
    queries: adminOrgIds.map((orgId) => ({
      queryKey: ["api-keys", orgId] as const,
      queryFn: async (): Promise<readonly ApiKey[]> => {
        const res = await api.get<{ readonly keys: readonly ApiKey[] }>(
          `/api-keys?org_id=${encodeURIComponent(orgId)}`,
        );
        return res.keys;
      },
    })),
  });

  const orgKeys = useMemo(() => {
    const out: ApiKey[] = [];
    for (const q of orgQueries) {
      if (q.data) out.push(...q.data);
    }
    return out;
  }, [orgQueries]);

  const merged = useMemo(() => {
    const personalKeys = personal.data ?? [];
    // Personal first, then org keys in the same admin-org order as useOrgs.
    const byId = new Map<string, ApiKey>();
    for (const k of personalKeys) byId.set(k.id, k);
    for (const k of orgKeys) if (!byId.has(k.id)) byId.set(k.id, k);
    return Array.from(byId.values());
  }, [personal.data, orgKeys]);

  const isLoading = personal.isLoading;
  const isFetching = personal.isFetching || orgQueries.some((q) => q.isFetching);

  return {
    data: merged,
    isLoading,
    isFetching,
    error: personal.error,
  } as const;
}

export function useApiKey(keyId: string) {
  return useQuery({
    queryKey: ["api-keys", keyId],
    queryFn: async (): Promise<ApiKey> => {
      return api.get<ApiKey>(`/api-keys/${keyId}`);
    },
    enabled: Boolean(keyId),
  });
}

export function useApiKeysUsage(days = 7) {
  return useQuery({
    queryKey: ["api-keys", "usage", days],
    queryFn: async (): Promise<readonly ApiKeyUsage[]> => {
      const res = await api.get<ApiKeyUsageListResponse>(`/api-keys/usage?days=${String(days)}`);
      return res.usage;
    },
  });
}

/**
 * Aggregate personal + every admined-org usage list into a single array.
 *
 * Mirrors `useAllAdminedApiKeys` exactly: fires one usage query per scope
 * (personal + each org the caller admins) in parallel, then merges by
 * `api_key_id`. The Usage Dashboard renders as soon as the personal scope
 * resolves; org-scope cards fill in progressively. Individual org-scope
 * errors are swallowed so a single failed org does not blank the whole
 * dashboard, matching the table's failure mode.
 *
 * Backend support for the org_id query param was added in
 * ChronoAIProject/NyxID#542.
 */
export function useAllAdminedApiKeysUsage(days = 7) {
  const personal = useApiKeysUsage(days);
  const { data: orgs } = useOrgs();

  const adminOrgIds = useMemo(
    () => (orgs ?? []).filter((o) => o.your_role === "admin").map((o) => o.id),
    [orgs],
  );

  const orgQueries = useQueries({
    queries: adminOrgIds.map((orgId) => ({
      queryKey: ["api-keys", "usage", days, orgId] as const,
      queryFn: async (): Promise<readonly ApiKeyUsage[]> => {
        const res = await api.get<ApiKeyUsageListResponse>(
          `/api-keys/usage?days=${String(days)}&org_id=${encodeURIComponent(orgId)}`,
        );
        return res.usage;
      },
    })),
  });

  const orgUsage = useMemo(() => {
    const out: ApiKeyUsage[] = [];
    for (const q of orgQueries) {
      if (q.data) out.push(...q.data);
    }
    return out;
  }, [orgQueries]);

  const merged = useMemo(() => {
    const byId = new Map<string, ApiKeyUsage>();
    for (const u of personal.data ?? []) byId.set(u.api_key_id, u);
    for (const u of orgUsage) if (!byId.has(u.api_key_id)) byId.set(u.api_key_id, u);
    return Array.from(byId.values());
  }, [personal.data, orgUsage]);

  return {
    data: merged,
    isLoading: personal.isLoading,
    isFetching: personal.isFetching || orgQueries.some((q) => q.isFetching),
    error: personal.error,
  } as const;
}

export function useApiKeyUsage(keyId: string, days = 7) {
  return useQuery({
    queryKey: ["api-keys", keyId, "usage", days],
    queryFn: async (): Promise<ApiKeyUsage> => {
      return api.get<ApiKeyUsage>(`/api-keys/${keyId}/usage?days=${String(days)}`);
    },
    enabled: Boolean(keyId),
  });
}

interface CreateApiKeyPayload {
  readonly name: string;
  readonly scopes: string;
  readonly expires_at: string | null;
  readonly description?: string;
  readonly allowed_service_ids?: readonly string[];
  readonly allowed_node_ids?: readonly string[];
  readonly allow_all_services?: boolean;
  readonly allow_all_nodes?: boolean;
  readonly callback_url?: string;
  /** Create the key under the given org (caller must be an org admin). */
  readonly target_org_id?: string;
}

export function useCreateApiKey() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (
      data: CreateApiKeyFormData,
    ): Promise<ApiKeyCreateResponse> => {
      // Backend expects scopes as a space-separated string, not an array.
      // expires_at must be ISO 8601 datetime (not bare date like "2026-03-25").
      const allowAllServices = data.allow_all_services ?? true;
      const allowAllNodes = data.allow_all_nodes ?? true;
      const payload: CreateApiKeyPayload = {
        name: data.name,
        scopes: data.scopes.join(" "),
        expires_at: data.expires_at
          ? new Date(`${data.expires_at}T23:59:59Z`).toISOString()
          : null,
        description: data.description ?? undefined,
        allowed_service_ids: allowAllServices
          ? undefined
          : (data.allowed_service_ids ?? []),
        allowed_node_ids: allowAllNodes
          ? undefined
          : (data.allowed_node_ids ?? []),
        allow_all_services: allowAllServices,
        allow_all_nodes: allowAllNodes,
        callback_url: data.callback_url ?? undefined,
        target_org_id: data.target_org_id ?? undefined,
      };
      return api.post<ApiKeyCreateResponse>("/api-keys", payload);
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({
        predicate: (q) =>
          Array.isArray(q.queryKey) && q.queryKey[0] === "api-keys",
      });
    },
  });
}

interface UpdateApiKeyParams {
  readonly keyId: string;
  readonly name?: string;
  readonly description?: string;
  readonly scopes?: string;
  readonly allowed_service_ids?: readonly string[];
  readonly allowed_node_ids?: readonly string[];
  readonly allow_all_services?: boolean;
  readonly allow_all_nodes?: boolean;
  readonly platform?: string | null;
  readonly callback_url?: string | null;
  readonly rate_limit_per_second?: number | null;
  readonly rate_limit_burst?: number | null;
}

export function useUpdateApiKey() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (params: UpdateApiKeyParams): Promise<ApiKey> => {
      const { keyId, ...body } = params;
      return api.put<ApiKey>(`/api-keys/${keyId}`, body);
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({
        predicate: (q) =>
          Array.isArray(q.queryKey) && q.queryKey[0] === "api-keys",
      });
    },
  });
}

export function useDeleteApiKey() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (id: string): Promise<void> => {
      return api.delete<void>(`/api-keys/${id}`);
    },
    onSuccess: () => {
      // Invalidate both the personal scope and every org-scope cache.
      // `predicate` catches keys like `["api-keys", "org", <orgId>]` that
      // `useAllAdminedApiKeys` populates lazily.
      void queryClient.invalidateQueries({
        predicate: (q) =>
          Array.isArray(q.queryKey) && q.queryKey[0] === "api-keys",
      });
    },
  });
}

export function useRotateApiKey() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (id: string): Promise<ApiKeyCreateResponse> => {
      return api.post<ApiKeyCreateResponse>(`/api-keys/${id}/rotate`);
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({
        predicate: (q) =>
          Array.isArray(q.queryKey) && q.queryKey[0] === "api-keys",
      });
    },
  });
}
