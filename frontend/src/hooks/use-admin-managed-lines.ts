import {
  useInfiniteQuery,
  useMutation,
  useQueryClient,
} from "@tanstack/react-query";
import { api, apiClient } from "@/lib/api-client";
import { useAuthStore } from "@/stores/auth-store";
import {
  adminManagedLinePageSchema,
  managedAccountPageSchema,
  managedEffectPageSchema,
  managedEligibilityPageSchema,
  managedLineOperationSchema,
  managedLinePreparationSchema,
  managedReportSchema,
} from "@/schemas/admin-managed-lines";
import { managedLifecycleSchema } from "@/schemas/managed-lines";

function guard(actor: string | undefined) {
  if (!actor || actor !== useAuthStore.getState().user?.id)
    throw new Error("Account changed. Reload this page.");
}
const keys = {
  all: (actor: string | undefined) => ["admin", "managed-lines", actor] as const,
};
const page = (path: string, after: string) =>
  `${path}${after ? `?after=${encodeURIComponent(after)}` : ""}`;
/** Skipped rows accumulate across every loaded page, once each. */
const unreadable = (pages: readonly { unreadable_ids?: string[] }[] | undefined) => [
  ...new Set(pages?.flatMap((p) => p.unreadable_ids ?? []) ?? []),
];

export function useAdminManagedLines() {
  const actor = useAuthStore((s) => s.user?.id);
  const lines = useInfiniteQuery({
    queryKey: [...keys.all(actor), "lines"],
    retry: false,
    initialPageParam: "",
    getNextPageParam: (p: { next_after_id?: string | null }) =>
      p.next_after_id ?? undefined,
    queryFn: async ({ pageParam }) =>
      adminManagedLinePageSchema.parse(
        await api.get(page("/admin/managed-lines", pageParam), {
          authorityGuard: () => guard(actor),
        }),
      ),
  });
  const accounts = useInfiniteQuery({
    queryKey: [...keys.all(actor), "accounts"],
    retry: false,
    initialPageParam: "",
    getNextPageParam: (p: { next_after_id?: string | null }) =>
      p.next_after_id ?? undefined,
    queryFn: async ({ pageParam }) =>
      managedAccountPageSchema.parse(
        await api.get(page("/admin/managed-lines/accounts", pageParam), {
          authorityGuard: () => guard(actor),
        }),
      ),
  });
  const eligibility = useInfiniteQuery({
    queryKey: [...keys.all(actor), "eligibility"],
    retry: false,
    initialPageParam: "",
    getNextPageParam: (p: { next_after_id?: string | null }) =>
      p.next_after_id ?? undefined,
    queryFn: async ({ pageParam }) =>
      managedEligibilityPageSchema.parse(
        await api.get(page("/admin/managed-comms/eligibility", pageParam), {
          authorityGuard: () => guard(actor),
        }),
      ),
  });
  return {
    lines: {
      ...lines,
      data: lines.data?.pages.flatMap((p) => p.lines),
      unreadableIds: unreadable(lines.data?.pages),
    },
    premiumEnabled: lines.data?.pages[0]?.premium_enabled ?? false,
    accounts: {
      ...accounts,
      data: accounts.data?.pages.flatMap((p) => p.accounts),
      unreadableIds: unreadable(accounts.data?.pages),
    },
    eligibility: {
      ...eligibility,
      data: eligibility.data?.pages.flatMap((p) => p.eligibility),
    },
  };
}

type Write = readonly [endpoint: string, method: "POST" | "PUT", body: unknown];

/** Admin writes are revision-fenced and never retried automatically. */
function useAdminWrite<I, T = unknown>(
  build: (input: I) => Write,
  parse: (value: unknown) => T = (value) => value as T,
) {
  const actor = useAuthStore((s) => s.user?.id);
  const client = useQueryClient();
  return useMutation({
    retry: false,
    mutationFn: async (input: I) => {
      guard(actor);
      const [endpoint, method, body] = build(input);
      return parse(
        await apiClient<unknown>(endpoint, {
          method,
          body,
          authorityGuard: () => guard(actor),
        }),
      );
    },
    onSettled: () => client.invalidateQueries({ queryKey: keys.all(actor) }),
  });
}
const id = encodeURIComponent;
const operation = (path: string) => (i: { lineId: string; body: unknown }): Write => [
  `/admin/managed-lines/${id(i.lineId)}/${path}`,
  "POST",
  i.body,
];
const parseOperation = (v: unknown) => managedLineOperationSchema.parse(v);

export function useAdminManagedLineActions() {
  return {
    prepare: useAdminWrite<void, ReturnType<typeof managedLinePreparationSchema.parse>>(
      (): Write => ["/admin/managed-lines/prepare", "POST", {}],
      (v) => managedLinePreparationSchema.parse(v),
    ),
    register: useAdminWrite((body: unknown): Write => ["/admin/managed-lines", "POST", body]),
    update: useAdminWrite((i: { lineId: string; body: unknown }): Write => [
      `/admin/managed-lines/${id(i.lineId)}`,
      "PUT",
      i.body,
    ]),
    suspend: useAdminWrite(operation("suspend"), parseOperation),
    resume: useAdminWrite(operation("resume"), parseOperation),
    retire: useAdminWrite(operation("retire"), parseOperation),
    approve: useAdminWrite((i: { lineId: string; revision: number }): Write => [
      `/admin/managed-lines/${id(i.lineId)}/approve-reassignment`,
      "POST",
      { revision: i.revision },
    ]),
    release: useAdminWrite(
      (body: { request_id: string; line_id: string; claim_id: string }): Write => [
        "/admin/managed-lines/releases",
        "POST",
        body,
      ],
      (v) => managedLifecycleSchema.parse(v),
    ),
    voices: useAdminWrite((i: { lineId: string; body: unknown }): Write => [
      `/admin/managed-lines/${id(i.lineId)}/standard-voices`,
      "PUT",
      i.body,
    ]),
    costPolicy: useAdminWrite((i: { lineId: string; body: unknown }): Write => [
      `/admin/managed-lines/${id(i.lineId)}/cost-policy`,
      "PUT",
      i.body,
    ]),
    saveEligibility: useAdminWrite((i: { ownerId: string; body: unknown }): Write => [
      `/admin/managed-comms/eligibility/${id(i.ownerId)}`,
      "PUT",
      i.body,
    ]),
    applyEligibility: useAdminWrite((i: { ownerId: string; revision: number }): Write => [
      `/admin/managed-comms/eligibility/${id(i.ownerId)}/apply`,
      "POST",
      { revision: i.revision },
    ]),
    revokeEligibility: useAdminWrite((i: { ownerId: string; revision: number }): Write => [
      `/admin/managed-comms/eligibility/${id(i.ownerId)}/revoke`,
      "POST",
      { revision: i.revision },
    ]),
    rotate: useAdminWrite(
      (i: { accountId: string; revision: number; credential: string }): Write => [
        `/admin/managed-comms/accounts/${id(i.accountId)}/rotate`,
        "POST",
        { expected_revision: i.revision, credential: i.credential },
      ],
    ),
    underwriting: useAdminWrite((i: { accountId: string; body: unknown }): Write => [
      `/admin/managed-lines/accounts/${id(i.accountId)}/underwriting`,
      "PUT",
      i.body,
    ]),
    voiceAccount: useAdminWrite((i: { accountId: string; body: unknown }): Write => [
      `/admin/managed-lines/accounts/${id(i.accountId)}/premium`,
      "PUT",
      i.body,
    ]),
    premiumSwitch: useAdminWrite((enabled: boolean): Write => [
      "/admin/managed-lines/premium-switch",
      "PUT",
      { enabled },
    ]),
    resolveEvidence: useAdminWrite(
      (i: { sessionId: string; seconds: number; reference: string }): Write => [
        `/admin/managed-lines/calls/${id(i.sessionId)}/resolve-evidence`,
        "POST",
        { provider_seconds: i.seconds, provider_reference: i.reference },
      ],
    ),
  };
}

/** Admin/operator report: UTC midnight window of at most 31 days. */
export function useManagedLinesReport(range: { from: string; until: string; lineId?: string }) {
  const actor = useAuthStore((s) => s.user?.id);
  return useInfiniteQuery({
    queryKey: ["admin", "managed-lines-report", actor, range],
    retry: false,
    initialPageParam: "",
    getNextPageParam: (p: { next_after: string | null }) => p.next_after ?? undefined,
    queryFn: async ({ pageParam }) => {
      const params = new URLSearchParams({ from: range.from, until: range.until });
      if (range.lineId) params.set("line_id", range.lineId);
      if (pageParam) params.set("after", pageParam);
      return managedReportSchema.parse(
        await api.get(`/admin/managed-lines/report?${params}`, {
          authorityGuard: () => guard(actor),
        }),
      );
    },
  });
}

export function useManagedLineEffects(
  range: { from: string; until: string; lineId: string } | null,
) {
  const actor = useAuthStore((s) => s.user?.id);
  return useInfiniteQuery({
    queryKey: ["admin", "managed-lines-effects", actor, range],
    enabled: range != null,
    retry: false,
    initialPageParam: "",
    getNextPageParam: (p: { next_after: string | null }) => p.next_after ?? undefined,
    queryFn: async ({ pageParam }) => {
      const params = new URLSearchParams({
        line_id: range!.lineId,
        from: range!.from,
        until: range!.until,
      });
      if (pageParam) params.set("after", pageParam);
      return managedEffectPageSchema.parse(
        await api.get(`/admin/managed-lines/report/effects?${params}`, {
          authorityGuard: () => guard(actor),
        }),
      );
    },
  });
}
