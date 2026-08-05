import { useEffect, useRef, useState } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import { connectWatchInterval } from "@/lib/assistant/connect-watch";
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

/** Terminal states of a placeholder key created for an out-of-band flow. */
export const KEY_AUTH_ACTIVE = "active";
export const KEY_AUTH_FAILED = "failed";

/**
 * Poll one key while an out-of-band authorization finishes elsewhere.
 *
 * OAuth hands the user to the provider in a separate tab, so nothing in this
 * tab observes the callback. Polling the placeholder key is how we learn the
 * flow ended: `pending_auth` → `active` on success, `failed` when the user
 * denies or abandons it (the backend's lazy placeholder reconciliation
 * converges abandoned flows to `failed`).
 *
 * Shares `["keys", keyId]` with `useKey` so a completed flow warms the same
 * cache entry. Polls only while `enabled` — never leave this running behind a
 * closed dialog or a card that already resolved.
 */
export function useKeyAuthorizationStatus(
  keyId: string | null,
  enabled: boolean,
  previousAuthorizationAt?: string | null,
) {
  const queryClient = useQueryClient();
  const active = Boolean(keyId) && enabled;
  const query = useQuery({
    queryKey: ["keys", keyId],
    queryFn: async (): Promise<KeyInfo> => {
      return api.get<KeyInfo>(`/keys/${keyId ?? ""}`);
    },
    enabled: active,
    // 2 s while waiting on a human in another tab; refetch the moment they
    // come back so the card flips without waiting out the interval. Stops
    // dead once the row is terminal — a caller that leaves its dialog open
    // must not keep hitting `/keys/:id` forever.
    refetchInterval: (current) => {
      const key = current.state.data;
      const authorizationAdvanced =
        previousAuthorizationAt === undefined ||
        (key?.last_authorized_at != null &&
          key.last_authorized_at !== previousAuthorizationAt);
      if (
        key?.status === KEY_AUTH_FAILED ||
        (key?.status === KEY_AUTH_ACTIVE && authorizationAdvanced)
      ) {
        return false;
      }
      return active ? 2_000 : false;
    },
    refetchOnWindowFocus: active,
  });

  // This query watches ONE key; the assistant's connect card reads the
  // `["keys"]` list. Without this the dialog can say Connected while the
  // transcript card is still Authorizing, because nothing refreshes the list
  // the card renders from. `exact` keeps it off this query's own key.
  const status = query.data?.status;
  const authorizationAdvanced =
    previousAuthorizationAt === undefined ||
    (query.data?.last_authorized_at != null &&
      query.data.last_authorized_at !== previousAuthorizationAt);
  useEffect(() => {
    if (
      status === KEY_AUTH_FAILED ||
      (status === KEY_AUTH_ACTIVE && authorizationAdvanced)
    ) {
      void queryClient.invalidateQueries({ queryKey: ["keys"], exact: true });
    }
  }, [authorizationAdvanced, status, queryClient]);

  return query;
}

export interface KeyAuthorizationWatch {
  /** Latest observed key status; `undefined` before the first read lands. */
  readonly status: string | undefined;
  /** Backend-supplied reason when `status` is `failed`. */
  readonly errorMessage: string | undefined;
  /** The watch gave up before the row reached a terminal status. */
  readonly timedOut: boolean;
}

/**
 * Watch a placeholder key to a terminal status in the background.
 *
 * The difference from `useKeyAuthorizationStatus` is ownership: that hook is
 * scoped to an open dialog and stops when the dialog closes. This one is
 * scoped to a *card*, which outlives the dialog, because closing the dialog is
 * the normal path — the user tabs away to the provider and comes back to the
 * chat, not to the dialog. It is the browser's `wait_for_authorized_key`.
 *
 * Two things bound it: `enabled` (the caller's presence gate — do not poll for
 * a tab nobody is looking at) and `deadlineAt` (the give-up time, which the
 * caller extends while the user is active). Past the deadline it stops polling
 * and reports `timedOut` so the card can say so instead of waiting silently.
 */
export function useKeyAuthorizationWatch(
  keyId: string | null,
  options: { readonly enabled: boolean; readonly deadlineAt: number },
): KeyAuthorizationWatch {
  const { enabled, deadlineAt } = options;
  const queryClient = useQueryClient();
  /**
   * The (key, deadline) pair a timer has already fired for. Storing the pair
   * rather than a boolean means a new key or a deadline pushed out by fresh
   * activity un-expires the watch by comparison alone — no reset effect, and
   * no state written synchronously from an effect.
   */
  const [expiredFor, setExpiredFor] = useState<{
    readonly keyId: string;
    readonly deadlineAt: number;
  } | null>(null);
  const startedAtRef = useRef(0);

  // A new key restarts the cadence window. Ref-only, so render stays pure.
  useEffect(() => {
    startedAtRef.current = Date.now();
  }, [keyId]);

  // Nothing re-renders on its own when a deadline passes, so arm a timer for
  // it. Re-armed whenever activity pushes `deadlineAt` out.
  useEffect(() => {
    if (!enabled || !keyId || deadlineAt <= 0) return;
    const fire = () => setExpiredFor({ keyId, deadlineAt });
    // Zero delay rather than a direct call: an effect that sets state inline
    // re-enters render before the browser can paint.
    const timer = setTimeout(fire, Math.max(0, deadlineAt - Date.now()));
    return () => clearTimeout(timer);
  }, [enabled, keyId, deadlineAt]);

  const expired =
    expiredFor !== null &&
    expiredFor.keyId === keyId &&
    expiredFor.deadlineAt >= deadlineAt;
  const active = Boolean(keyId) && enabled && !expired;
  const query = useQuery({
    queryKey: ["keys", keyId],
    queryFn: async (): Promise<KeyInfo> => {
      return api.get<KeyInfo>(`/keys/${keyId ?? ""}`);
    },
    enabled: active,
    refetchInterval: (current) => {
      const status = current.state.data?.status;
      if (status === KEY_AUTH_ACTIVE || status === KEY_AUTH_FAILED) {
        return false;
      }
      if (!active) return false;
      return connectWatchInterval(startedAtRef.current, Date.now());
    },
    // The whole point of the background watch: returning from the provider's
    // tab resolves the card at once rather than after the next interval.
    refetchOnWindowFocus: active,
  });

  const status = query.data?.status;
  useEffect(() => {
    if (status === KEY_AUTH_ACTIVE || status === KEY_AUTH_FAILED) {
      void queryClient.invalidateQueries({ queryKey: ["keys"], exact: true });
    }
  }, [status, queryClient]);

  const terminal = status === KEY_AUTH_ACTIVE || status === KEY_AUTH_FAILED;
  return {
    status,
    errorMessage: query.data?.error_message ?? undefined,
    timedOut: expired && !terminal,
  };
}

interface UseCatalogOptions {
  readonly includeAll?: boolean;
}

export function useCatalog(options: UseCatalogOptions = {}) {
  const includeAll = options.includeAll ?? false;

  return useQuery({
    queryKey: includeAll ? ["catalog", { includeAll: true }] : ["catalog"],
    queryFn: async (): Promise<readonly CatalogEntry[]> => {
      const res = await api.get<CatalogListResponse>(
        includeAll ? "/catalog?include_all=true" : "/catalog",
      );
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
  readonly admin_only?: boolean;
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
    mutationFn: async (
      input: string | DeleteKeyInput,
    ): Promise<DeleteKeyResponse> => {
      const params = typeof input === "string" ? { keyId: input } : input;
      const query = new URLSearchParams();
      if (params.cascadeGrant) query.set("cascade_grant", "true");
      if (params.grantScope) query.set("grant_scope", params.grantScope);
      const suffix = query.size > 0 ? `?${query.toString()}` : "";
      return api.delete<DeleteKeyResponse>(`/keys/${params.keyId}${suffix}`);
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["keys"] });
      void queryClient.invalidateQueries({ queryKey: ["llm-status"] });
    },
  });
}

export interface DeleteKeyInput {
  readonly keyId: string;
  readonly cascadeGrant?: boolean;
  readonly grantScope?: "token";
}

export interface DeleteKeyResponse {
  readonly message: string;
  readonly deleted: boolean;
  readonly upstream_revocation_scheduled: boolean;
}

interface UpdateKeyParams {
  readonly keyId: string;
  readonly label?: string;
  readonly endpoint_url?: string;
  readonly auth_method?: string;
  readonly auth_key_name?: string;
  readonly node_id?: string;
  readonly is_active?: boolean;
  readonly admin_only?: boolean;
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
  /** Empty array clears the existing list. */
  readonly recommended_skills?: readonly string[];
}

export function useUpdateEndpoint() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (params: UpdateEndpointParams): Promise<void> => {
      return api.put<void>(`/endpoints/${params.endpointId}`, {
        url: params.url,
        label: params.label,
        openapi_spec_url: params.openapi_spec_url,
        recommended_skills: params.recommended_skills,
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
  readonly admin_only?: boolean;
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
