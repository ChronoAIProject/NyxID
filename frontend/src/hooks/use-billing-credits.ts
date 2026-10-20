import {
  allocationFormSchema,
  allocationSummarySchema,
  type AllocationForm,
} from "@/schemas/billing-credits";
import {
  allowanceBundleFormSchema,
  allowanceBundleResponseSchema,
  type AllowanceBundleForm,
} from "@/schemas/billing-credits";
import { normalizedBillingTargets } from "@/lib/billing-targets";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import {
  allowanceFormSchema,
  adminCreditGrantListSchema,
  creditGrantListSchema,
  creditGrantSchema,
  creditScheduleListSchema,
  creditScheduleSchema,
  issueGrantFormSchema,
  issueGrantResponseSchema,
  scheduleFormSchema,
  usageAllowanceListSchema,
  usageAllowanceSchema,
  userAllowanceListSchema,
  type AllowanceForm,
  type IssueGrantForm,
  type ScheduleForm,
} from "@/schemas/billing-credits";

const ADMIN_CREDITS_KEY = ["admin", "credits"] as const;
const USER_CREDITS_KEY = ["billing", "credits"] as const;

function benefitPath(path: "grants" | "allowances", ownerId?: string): string {
  return ownerId
    ? `/billing/${path}?owner_id=${encodeURIComponent(ownerId)}`
    : `/billing/${path}`;
}

export function useAdminCreditGrants(page = 1, perPage = 50) {
  return useQuery({
    queryKey: [...ADMIN_CREDITS_KEY, "grants", page, perPage],
    refetchInterval: 30_000,
    refetchIntervalInBackground: false,
    queryFn: async () =>
      adminCreditGrantListSchema.parse(
        await api.get<unknown>(
          `/admin/credits/grants?page=${String(page)}&per_page=${String(perPage)}`,
        ),
      ),
  });
}

export function useIssueCreditGrant() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async (form: IssueGrantForm) => {
      const value = issueGrantFormSchema.parse(form);
      return issueGrantResponseSchema.parse(
        await api.post<unknown>("/admin/credits/grants", {
          ...value,
          ...normalizedBillingTargets(value),
          service_refs: value.all_services ? [] : value.service_refs,
          expires_at: value.expires_at
            ? new Date(value.expires_at).toISOString()
            : null,
          reason: value.reason || null,
        }),
      );
    },
    onSuccess: () =>
      void queryClient.invalidateQueries({ queryKey: ADMIN_CREDITS_KEY }),
  });
}

export function useRevokeCreditGrant() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async (grantId: string | { id: string; allocation: boolean }) =>
      creditGrantSchema.parse(
        await api.delete<unknown>(
          `/admin/credits/${typeof grantId === "string" ? "grants" : "allocations"}/${encodeURIComponent(typeof grantId === "string" ? grantId : grantId.id)}`,
        ),
      ),
    onSuccess: () =>
      void queryClient.invalidateQueries({ queryKey: ADMIN_CREDITS_KEY }),
  });
}

export function useAdminCreditSchedules() {
  return useQuery({
    queryKey: [...ADMIN_CREDITS_KEY, "schedules"],
    queryFn: async () =>
      creditScheduleListSchema.parse(
        await api.get<unknown>("/admin/credits/schedules"),
      ),
  });
}

export function useCreateCreditSchedule() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async (form: ScheduleForm) => {
      const value = scheduleFormSchema.parse(form);
      return creditScheduleSchema.parse(
        await api.post<unknown>("/admin/credits/schedules", {
          ...value,
          ...normalizedBillingTargets(value),
          service_refs: value.all_services ? [] : value.service_refs,
          reason: value.reason || null,
        }),
      );
    },
    onSuccess: () =>
      void queryClient.invalidateQueries({ queryKey: ADMIN_CREDITS_KEY }),
  });
}

export type UpdateCreditScheduleBody = Partial<
  Omit<ScheduleForm, "recurrence">
> & { readonly is_active?: boolean };

export function useUpdateCreditSchedule() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async ({
      id,
      body,
    }: {
      readonly id: string;
      readonly body: UpdateCreditScheduleBody;
    }) =>
      creditScheduleSchema.parse(
        await api.patch<unknown>(
          `/admin/credits/${body.service_limits !== undefined ? "allocations/schedules" : "schedules"}/${encodeURIComponent(id)}`,
          body,
        ),
      ),
    onSuccess: () =>
      void queryClient.invalidateQueries({ queryKey: ADMIN_CREDITS_KEY }),
  });
}

export function useAdminAllowances() {
  return useQuery({
    queryKey: [...ADMIN_CREDITS_KEY, "allowances"],
    queryFn: async () =>
      usageAllowanceListSchema.parse(
        await api.get<unknown>("/admin/credits/allowances"),
      ),
  });
}

export function useCreateAllowance() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async (form: AllowanceForm) => {
      const value = allowanceFormSchema.parse(form);
      return usageAllowanceSchema.parse(
        await api.post<unknown>("/admin/credits/allowances", {
          ...value,
          ...normalizedBillingTargets(value),
        }),
      );
    },
    onSuccess: () =>
      void queryClient.invalidateQueries({ queryKey: ADMIN_CREDITS_KEY }),
  });
}

export function useUpdateAllowance() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async ({
      id,
      body,
    }: {
      readonly id: string;
      readonly body: Partial<AllowanceForm> & { readonly is_active?: boolean };
    }) =>
      usageAllowanceSchema.parse(
        await api.patch<unknown>(
          `/admin/credits/allowances/${encodeURIComponent(id)}`,
          body,
        ),
      ),
    onSuccess: () =>
      void queryClient.invalidateQueries({ queryKey: ADMIN_CREDITS_KEY }),
  });
}

export function useActiveCreditGrants(ownerId?: string) {
  return useQuery({
    queryKey: [...USER_CREDITS_KEY, "grants", ownerId ?? "personal"],
    queryFn: async () =>
      creditGrantListSchema.parse(
        await api.get<unknown>(benefitPath("grants", ownerId)),
      ),
    retry: false,
  });
}

export function useCurrentAllowances(ownerId?: string) {
  return useQuery({
    queryKey: [...USER_CREDITS_KEY, "allowances", ownerId ?? "personal"],
    queryFn: async () =>
      userAllowanceListSchema.parse(
        await api.get<unknown>(benefitPath("allowances", ownerId)),
      ),
    retry: false,
  });
}

export function useCreateAllowanceBundle() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: async (form: AllowanceBundleForm) =>
      allowanceBundleResponseSchema.parse(
        await api.post<unknown>(
          "/admin/credits/allowances",
          allowanceBundleFormSchema.parse({
            ...form,
            ...normalizedBillingTargets(form),
          }),
        ),
      ),
    onSuccess: () =>
      void client.invalidateQueries({ queryKey: ADMIN_CREDITS_KEY }),
  });
}
export function useReplaceAllowanceBundle() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: async ({
      id,
      body,
    }: {
      id: string;
      body: AllowanceBundleForm;
    }) =>
      allowanceBundleResponseSchema.parse(
        await api.put<unknown>(
          `/admin/credits/allowances/bundles/${encodeURIComponent(id)}`,
          allowanceBundleFormSchema.parse({
            ...body,
            ...normalizedBillingTargets(body),
          }),
        ),
      ),
    onSuccess: () =>
      void client.invalidateQueries({ queryKey: ADMIN_CREDITS_KEY }),
  });
}
export function useSetAllowanceBundleActive() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: async ({
      id,
      body,
    }: {
      id: string;
      body: { is_active: boolean };
    }) =>
      allowanceBundleResponseSchema.parse(
        await api.patch<unknown>(
          `/admin/credits/allowances/bundles/${encodeURIComponent(id)}`,
          body,
        ),
      ),
    onSuccess: () =>
      void client.invalidateQueries({ queryKey: ADMIN_CREDITS_KEY }),
  });
}

export function useAllocationOverview() {
  return useQuery({
    queryKey: [...ADMIN_CREDITS_KEY, "allocation-summary"],
    queryFn: async () =>
      allocationSummarySchema.parse(
        await api.get<unknown>("/admin/credits/allocations/summary"),
      ),
    retry: false,
    refetchInterval: 30_000,
    refetchIntervalInBackground: false,
  });
}

/** New routes fail on older servers instead of silently dropping policies. */
export function useAllocateCredits() {
  const client = useQueryClient();
  return useMutation({
    mutationFn: async ({
      form,
      requestId,
    }: {
      form: AllocationForm;
      requestId: string;
    }) => {
      const value = allocationFormSchema.parse(form);
      const { recurrence, expires_at, expiry, ...policy } = value;
      const body = {
        ...policy,
        ...normalizedBillingTargets(value),
        amount_credits: 0,
        service_refs: value.all_services ? [] : value.service_refs,
        reason: value.reason || null,
        request_id: requestId,
      };
      return recurrence === "one_time"
        ? {
            kind: "one_time" as const,
            result: issueGrantResponseSchema.parse(
              await api.post<unknown>("/admin/credits/allocations", {
                ...body,
                expires_at: expires_at
                  ? new Date(expires_at).toISOString()
                  : null,
              }),
            ),
          }
        : {
            kind: "recurring" as const,
            result: creditScheduleSchema.parse(
              await api.post<unknown>("/admin/credits/allocations/schedules", {
                ...body,
                recurrence,
                expiry,
              }),
            ),
          };
    },
    onSuccess: () => {
      void client.invalidateQueries({ queryKey: ADMIN_CREDITS_KEY });
      void client.invalidateQueries({ queryKey: USER_CREDITS_KEY });
    },
  });
}
