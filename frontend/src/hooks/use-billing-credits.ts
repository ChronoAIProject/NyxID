import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import {
  allowanceFormSchema,
  adminCreditGrantListSchema,
  creditGrantListSchema,
  creditGrantSchema,
  issueGrantFormSchema,
  issueGrantResponseSchema,
  usageAllowanceListSchema,
  usageAllowanceSchema,
  userAllowanceListSchema,
  type AllowanceForm,
  type IssueGrantForm,
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
          target_user_ids:
            value.target_kind === "all_users" ? [] : value.target_user_ids,
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
    mutationFn: async (grantId: string) =>
      creditGrantSchema.parse(
        await api.delete<unknown>(
          `/admin/credits/grants/${encodeURIComponent(grantId)}`,
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
          target_user_ids:
            value.target_kind === "all_users" ? [] : value.target_user_ids,
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
