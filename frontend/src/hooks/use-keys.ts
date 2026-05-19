import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import type {
  KeyInfo,
  KeyListResponse,
  CatalogEntry,
  CatalogListResponse,
  ExternalApiKeyInfo,
  ExternalApiKeyListResponse,
} from "@/types/keys";
import type { DefaultRequestHeader } from "@/schemas/default-request-headers";
import type { WsFrameInjection } from "@/schemas/services";

// -- Queries --

export function useKeys() {
  return useQuery({
    queryKey: ["keys"],
    queryFn: async (): Promise<readonly KeyInfo[]> => {
      const res = await api.get<KeyListResponse>("/keys");
      return res.keys;
    },
  });
}

export function useKey(keyId: string) {
  return useQuery({
    queryKey: ["keys", keyId],
    queryFn: async (): Promise<KeyInfo> => {
      return api.get<KeyInfo>(`/keys/${keyId}`);
    },
    enabled: Boolean(keyId),
  });
}

export function useCatalog() {
  return useQuery({
    queryKey: ["catalog"],
    queryFn: async (): Promise<readonly CatalogEntry[]> => {
      const res = await api.get<CatalogListResponse>("/catalog");
      return res.entries;
    },
  });
}

/**
 * Fetch a single catalog entry by slug.
 *
 * Prefer this over scanning `useCatalog()` when you need to look up the
 * catalog entry for an already-provisioned key. The list endpoint
 * filters out no-auth / internal services that don't require credential
 * setup, but a key can still be backed by one of those catalog rows
 * (auto-provisioned). `/catalog/{slug}` returns the row regardless, so
 * inherited metadata (e.g. `default_request_headers`) stays visible for
 * those services — see NyxID#356 Codex review P2.
 *
 * Pass `null` or `undefined` when the key has no `catalog_service_slug`
 * (purely custom endpoint); the hook disables itself so no request is
 * issued.
 */
export function useCatalogEntry(slug: string | null | undefined) {
  return useQuery({
    queryKey: ["catalog", slug],
    queryFn: async (): Promise<CatalogEntry> => {
      return api.get<CatalogEntry>(`/catalog/${encodeURIComponent(slug!)}`);
    },
    enabled: Boolean(slug),
    staleTime: 60_000,
  });
}

export function useExternalApiKeys() {
  return useQuery({
    queryKey: ["external-api-keys"],
    queryFn: async (): Promise<readonly ExternalApiKeyInfo[]> => {
      const res = await api.get<ExternalApiKeyListResponse>("/api-keys/external");
      return res.api_keys;
    },
  });
}

// -- Mutations --

interface CreateKeyParams {
  readonly service_slug?: string;
  readonly credential?: string;
  readonly label: string;
  readonly endpoint_url?: string;
  readonly slug?: string;
  readonly auth_method?: string;
  readonly auth_key_name?: string;
  readonly node_id?: string;
  readonly service_type?: string;
  readonly ssh_host?: string;
  readonly ssh_port?: number;
  readonly ssh_certificate_auth?: boolean;
  readonly ssh_principals?: string;
  readonly ssh_certificate_ttl_minutes?: number;
  readonly openapi_spec_url?: string;
  readonly ws_frame_injections?: readonly WsFrameInjection[];
  /**
   * Create the service under the given org so every admin of that org can
   * manage the resulting UserService / UserEndpoint / UserApiKey rows.
   * Caller must be an admin of the target org.
   */
  readonly target_org_id?: string;
}

export function useCreateKey() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (params: CreateKeyParams): Promise<KeyInfo> => {
      return api.post<KeyInfo>("/keys", params);
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["keys"] });
      void queryClient.invalidateQueries({ queryKey: ["llm-status"] });
    },
  });
}

export function useDeleteKey() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (keyId: string): Promise<void> => {
      return api.delete<void>(`/keys/${keyId}`);
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["keys"] });
      void queryClient.invalidateQueries({ queryKey: ["llm-status"] });
    },
  });
}

interface UpdateKeyParams {
  readonly keyId: string;
  readonly label?: string;
  readonly endpoint_url?: string;
  readonly auth_method?: string;
  readonly auth_key_name?: string;
  readonly node_id?: string;
  readonly is_active?: boolean;
  /** Empty string clears the existing value. */
  readonly openapi_spec_url?: string;
  /**
   * NyxID#356 tri-state:
   *   `undefined` (omit) leaves unchanged,
   *   `null` clears,
   *   array replaces.
   */
  readonly default_request_headers?:
    | null
    | readonly DefaultRequestHeader[];
}

export function useUpdateKey() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (params: UpdateKeyParams): Promise<KeyInfo> => {
      const { keyId, ...body } = params;
      return api.put<KeyInfo>(`/keys/${keyId}`, body);
    },
    onSuccess: (_data, variables) => {
      void queryClient.invalidateQueries({ queryKey: ["keys"] });
      void queryClient.invalidateQueries({
        queryKey: ["keys", variables.keyId],
      });
    },
  });
}

interface UpdateEndpointParams {
  readonly endpointId: string;
  readonly url?: string;
  readonly label?: string;
  /** Empty string clears the existing value. */
  readonly openapi_spec_url?: string;
}

export function useUpdateEndpoint() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (params: UpdateEndpointParams): Promise<void> => {
      return api.put<void>(`/endpoints/${params.endpointId}`, {
        url: params.url,
        label: params.label,
        openapi_spec_url: params.openapi_spec_url,
      });
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["keys"] });
      void queryClient.invalidateQueries({ queryKey: ["external-api-keys"] });
    },
  });
}

interface UpdateUserServiceParams {
  readonly serviceId: string;
  readonly auth_method?: string;
  readonly auth_key_name?: string;
  readonly node_id?: string;
  readonly node_priority?: number;
  readonly is_active?: boolean;
  readonly custom_user_agent?: string;
  /**
   * NyxID#356 tri-state:
   *   `undefined` (omit) leaves unchanged,
   *   `null` clears,
   *   array replaces.
   */
  readonly default_request_headers?:
    | null
    | readonly DefaultRequestHeader[];
  readonly ws_frame_injections?: readonly WsFrameInjection[];
}

export function useUpdateUserService() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (params: UpdateUserServiceParams): Promise<void> => {
      const { serviceId, ...body } = params;
      return api.put<void>(`/user-services/${serviceId}`, body);
    },
    onSuccess: (_data, variables) => {
      void queryClient.invalidateQueries({ queryKey: ["keys"] });
      void queryClient.invalidateQueries({
        queryKey: ["keys", variables.serviceId],
      });
    },
  });
}

interface UpdateExternalApiKeyParams {
  readonly keyId: string;
  readonly label?: string;
  readonly credential?: string;
}

export function useUpdateExternalApiKey() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (params: UpdateExternalApiKeyParams): Promise<void> => {
      const { keyId, ...body } = params;
      return api.put<void>(`/api-keys/external/${keyId}`, body);
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["keys"] });
      void queryClient.invalidateQueries({ queryKey: ["external-api-keys"] });
    },
  });
}
