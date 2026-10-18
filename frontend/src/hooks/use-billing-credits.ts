import { useRef } from "react";
import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import {
  addCreditsFormSchema,
  allowanceFormSchema,
  adminCreditSummarySchema,
  adminCreditGrantListSchema,
  creditGrantListSchema,
  userAllowanceListSchema,
  creditGrantSchema,
  creditScheduleListSchema,
  creditScheduleSchema,
  issueGrantResponseSchema,
  usageAllowanceListSchema,
  usageAllowanceSchema,
  userCreditSummarySchema,
  type AllowanceForm,
  type AddCreditsForm,
  type ScheduleForm,
} from "@/schemas/billing-credits";

const ADMIN_CREDITS_KEY = ["admin", "credits"] as const;
const CREDIT_REFRESH_INTERVAL = 30_000;

const USER_CREDITS_KEY = ["billing", "credits"] as const;

export function useAdminCreditSummary() {
  return useQuery({
    refetchInterval: CREDIT_REFRESH_INTERVAL,
    refetchIntervalInBackground: false,
    queryKey: [...ADMIN_CREDITS_KEY, "summary"],
    queryFn: async () =>
      adminCreditSummarySchema.parse(
        await api.get<unknown>("/admin/credits/summary"),
      ),
  });
}

export function useUserCreditSummary(ownerId?: string) {
  return useQuery({
    refetchInterval: CREDIT_REFRESH_INTERVAL,
    refetchIntervalInBackground: false,
    queryKey: [...USER_CREDITS_KEY, "summary", ownerId ?? "personal"],
    queryFn: async () =>
      userCreditSummarySchema.parse(
        await api.get<unknown>(
          ownerId
            ? `/billing/credits?owner_id=${encodeURIComponent(ownerId)}`
            : "/billing/credits",
        ),
      ),
    retry: false,
  });
}

export function useAddCredits() {
  const queryClient = useQueryClient();
  const submission = useRef<{ signature: string; requestId: string } | null>(null);
  const mutation = useMutation({
    mutationFn: async (form: AddCreditsForm) => {
      const value = addCreditsFormSchema.parse(form);
      const body = {
        amount_credits: value.amount_credits,
        target_kind: value.target_kind,
        target_user_ids:
          value.target_kind === "all_users" ? [] : value.target_user_ids,
        all_services: value.all_services,
        service_refs: value.all_services ? [] : value.service_refs,
        reason: value.reason || null,
        service_limits: (value.service_limits ?? []).map(rule => ({ ...rule, label: rule.label || null })),
      };
      const signature = JSON.stringify({ ...body, recurrence: value.recurrence,
        expiry: value.recurrence === "one_time" ? value.expires_at || null : value.expiry });
      if (submission.current?.signature !== signature) submission.current = { signature, requestId: crypto.randomUUID() };
      const request_id = submission.current.requestId;
      if (value.recurrence === "one_time") {
        return issueGrantResponseSchema.parse(
          await api.post<unknown>("/admin/credits/grants", {
            ...body,
            request_id,
            expires_at: value.expires_at
              ? new Date(value.expires_at).toISOString()
              : null,
          }),
        );
      }
      return creditScheduleSchema.parse(
        await api.post<unknown>("/admin/credits/schedules", {
          ...body,
          request_id,
          recurrence: value.recurrence,
          expiry: value.expiry,
        }),
      );
    },
    onSuccess: () => {
      submission.current = null;
      void queryClient.invalidateQueries({ queryKey: ADMIN_CREDITS_KEY });
      void queryClient.invalidateQueries({ queryKey: USER_CREDITS_KEY });
    },
  });
  return { ...mutation, reset: () => { submission.current = null; mutation.reset(); } };
}

export function useAdminCreditGrants(page = 1, perPage = 50) {
  return useQuery({
    refetchInterval: CREDIT_REFRESH_INTERVAL,
    refetchIntervalInBackground: false,
    queryKey: [...ADMIN_CREDITS_KEY, "grants", page, perPage],
    queryFn: async () =>
      adminCreditGrantListSchema.parse(
        await api.get<unknown>(
          `/admin/credits/grants?page=${String(page)}&per_page=${String(perPage)}`,
        ),
      ),
  });
}

export function useRevokeCreditGrant() {
  const queryClient = useQueryClient();
  return useMutation({
    mutationFn: async (grantId: string) =>
      creditGrantSchema.parse(
        await api.delete<unknown>(
          `/admin/credits/grants/${encodeURIComponent(grantId)}`,
        ),
      ),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ADMIN_CREDITS_KEY });
      void queryClient.invalidateQueries({ queryKey: USER_CREDITS_KEY });
    },
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
          `/admin/credits/schedules/${encodeURIComponent(id)}`,
          body,
        ),
      ),
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ADMIN_CREDITS_KEY });
      void queryClient.invalidateQueries({ queryKey: USER_CREDITS_KEY });
    },
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
          target_user_ids:
            value.target_kind === "all_users" ? [] : value.target_user_ids,
        }),
      );
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ADMIN_CREDITS_KEY });
      void queryClient.invalidateQueries({ queryKey: USER_CREDITS_KEY });
    },
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
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ADMIN_CREDITS_KEY });
      void queryClient.invalidateQueries({ queryKey: USER_CREDITS_KEY });
    },
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

function benefitPath(path: "grants" | "allowances", ownerId?: string): string {
  return ownerId
    ? `/billing/${path}?owner_id=${encodeURIComponent(ownerId)}`
    : `/billing/${path}`;
}
