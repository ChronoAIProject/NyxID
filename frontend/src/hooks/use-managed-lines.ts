import {
  useInfiniteQuery,
  useMutation,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import { api, apiClient, ApiError } from "@/lib/api-client";
import { useAuthStore } from "@/stores/auth-store";
import {
  managedEligibilitySchema,
  managedLifecycleSchema,
  managedLineListSchema,
  managedQuoteSchema,
  type ManagedLine,
  type ManagedQuoteKind,
  type ManagedTerms,
  type ManagedVoiceOption,
} from "@/schemas/managed-lines";

function guard(actor: string | undefined) {
  if (!actor || actor !== useAuthStore.getState().user?.id)
    throw new Error("Account changed. Reopen NyxBot settings.");
}

export const managedLineKeys = {
  all: (actor: string | undefined) => ["managed-lines", actor] as const,
  eligibility: (actor: string | undefined) =>
    ["managed-lines", actor, "eligibility"] as const,
  lines: (actor: string | undefined) =>
    ["managed-lines", actor, "lines"] as const,
};

/** A missing eligibility record (404) means the merchant is not approved yet. */
export function useManagedEligibility(enabled = true) {
  const actor = useAuthStore((s) => s.user?.id);
  return useQuery({
    queryKey: managedLineKeys.eligibility(actor),
    enabled: enabled && !!actor,
    retry: false,
    queryFn: async () => {
      guard(actor);
      try {
        return managedEligibilitySchema.parse(
          await api.get("/managed-lines/eligibility", {
            authorityGuard: () => guard(actor),
          }),
        );
      } catch (error) {
        if (error instanceof ApiError && error.status === 404) return null;
        throw error;
      }
    },
  });
}

export function useManagedLines(enabled = true) {
  const actor = useAuthStore((s) => s.user?.id);
  const query = useInfiniteQuery({
    queryKey: managedLineKeys.lines(actor),
    enabled: enabled && !!actor,
    retry: false,
    initialPageParam: "",
    getNextPageParam: (page: { next_after_id?: string | null }) =>
      page.next_after_id ?? undefined,
    queryFn: async ({ pageParam }) => {
      guard(actor);
      return managedLineListSchema.parse(
        await api.get(
          `/managed-lines${pageParam ? `?after=${encodeURIComponent(pageParam)}` : ""}`,
          { authorityGuard: () => guard(actor) },
        ),
      );
    },
  });
  return {
    ...query,
    data: query.data?.pages.flatMap((p) => p.lines),
  };
}

export interface ClaimRequest {
  readonly request_id: string;
  readonly quote_id: string;
  readonly line_id: string;
  readonly agent_id: string;
  readonly voice_tier: "standard" | "premium";
  readonly standard_voice: ManagedVoiceOption | null;
  readonly enable_sms: boolean;
  readonly caller_confirmations: boolean;
  readonly auto_renew: boolean;
  readonly accepted_terms_versions: ManagedTerms;
}
export interface CoverageRequest {
  readonly request_id: string;
  readonly quote_id: string;
  readonly auto_renew: boolean;
  readonly accepted_terms_versions: ManagedTerms;
}

/**
 * Lifecycle writes are never retried automatically. Callers keep one request
 * ID per submission so a lost response can be replayed with the same payload.
 */
export function useManagedLineActions() {
  const actor = useAuthStore((s) => s.user?.id);
  const client = useQueryClient();
  const refresh = () =>
    client.invalidateQueries({ queryKey: managedLineKeys.all(actor) });
  const send = <T,>(
    endpoint: string,
    method: "POST" | "PUT",
    body: unknown,
  ) => {
    guard(actor);
    return apiClient<T>(endpoint, {
      method,
      body,
      authorityGuard: () => guard(actor),
    });
  };
  const quote = useMutation({
    retry: false,
    mutationFn: async (input: { lineId: string; kind: ManagedQuoteKind }) =>
      managedQuoteSchema.parse(
        await send(
          `/managed-lines/${encodeURIComponent(input.lineId)}/quotes`,
          "POST",
          { kind: input.kind },
        ),
      ),
  });
  const claim = useMutation({
    retry: false,
    mutationFn: async (input: ClaimRequest) =>
      managedLifecycleSchema.parse(
        await send("/managed-lines/claims", "POST", input),
      ),
    onSettled: refresh,
  });
  const coverage = useMutation({
    retry: false,
    mutationFn: async (input: CoverageRequest) =>
      managedLifecycleSchema.parse(
        await send("/managed-lines/coverage", "POST", input),
      ),
    onSettled: refresh,
  });
  const release = useMutation({
    retry: false,
    mutationFn: async (input: {
      request_id: string;
      line_id: string;
      claim_id: string;
    }) =>
      managedLifecycleSchema.parse(
        await send("/managed-lines/releases", "POST", input),
      ),
    onSettled: refresh,
  });
  const acceptTerms = useMutation({
    retry: false,
    mutationFn: (input: { revision: number; terms: ManagedTerms }) =>
      send<void>("/managed-lines/eligibility/terms", "POST", input),
    onSettled: refresh,
  });
  const setVoice = useMutation({
    retry: false,
    mutationFn: (input: {
      line: ManagedLine;
      option: ManagedVoiceOption;
    }) => {
      const claim = input.line.claim;
      if (!claim || claim.voice_binding_revision == null)
        throw new Error("Reload the line before changing its voice.");
      return send<void>(
        `/managed-lines/${encodeURIComponent(input.line.id)}/standard-voice`,
        "PUT",
        {
          claim_id: claim.id,
          binding_revision: claim.voice_binding_revision,
          option: input.option,
        },
      );
    },
    onSettled: refresh,
  });
  const confirmations = useMutation({
    retry: false,
    mutationFn: (input: { line: ManagedLine; enabled: boolean }) => {
      const claim = input.line.claim;
      if (!claim?.voice_binding_id || claim.voice_binding_revision == null)
        throw new Error("Reload the line before changing caller confirmations.");
      return send<void>(
        `/call-adapters/bindings/${encodeURIComponent(claim.voice_binding_id)}/caller-confirmations`,
        "PUT",
        {
          enabled: input.enabled,
          sms_binding_id: null,
          voice_revision: claim.voice_binding_revision,
        },
      );
    },
    onSettled: refresh,
  });
  return {
    quote,
    claim,
    coverage,
    release,
    acceptTerms,
    setVoice,
    confirmations,
  };
}
