import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import type {
  CreateOrgRequest,
  OrgListItem,
  OrgListResponse,
  OrgResponse,
  RedeemInviteResponse,
  SetPrimaryOrgRequest,
  UpdateOrgRequest,
} from "@/schemas/orgs";

// ─────────────────────────────────────────────────────────────────────────────
// Query keys
// ─────────────────────────────────────────────────────────────────────────────

const ORGS_ROOT = ["orgs"] as const;

export const orgsQueryKeys = {
  all: ORGS_ROOT,
  list: () => [...ORGS_ROOT, "list"] as const,
  detail: (orgId: string) => [...ORGS_ROOT, "detail", orgId] as const,
} as const;

// ─────────────────────────────────────────────────────────────────────────────
// Queries
// ─────────────────────────────────────────────────────────────────────────────

export function useOrgs() {
  return useQuery({
    queryKey: orgsQueryKeys.list(),
    queryFn: async (): Promise<readonly OrgListItem[]> => {
      const res = await api.get<OrgListResponse>("/orgs");
      return res.orgs;
    },
  });
}

export function useOrg(orgId: string) {
  return useQuery({
    queryKey: orgsQueryKeys.detail(orgId),
    queryFn: async (): Promise<OrgResponse> => {
      return api.get<OrgResponse>(`/orgs/${orgId}`);
    },
    enabled: Boolean(orgId),
  });
}

// ─────────────────────────────────────────────────────────────────────────────
// Mutations
// ─────────────────────────────────────────────────────────────────────────────

/**
 * Build a CREATE body by stripping both `undefined` and empty strings.
 * The backend validates `contact_email` with `validator::email`, so sending
 * `""` fails validation even though the field is optional. Use this for
 * `POST /orgs` only.
 */
function cleanCreateOrgRequest<T extends Record<string, unknown>>(input: T): T {
  const output: Record<string, unknown> = {};
  for (const [key, value] of Object.entries(input)) {
    if (value === "" || value === undefined) continue;
    output[key] = value;
  }
  return output as T;
}

/**
 * Build a PATCH body by stripping only `undefined` values, preserving empty
 * strings. The backend's `update_org_user` treats `avatar_url: ""` as
 * "clear this field" -- we must NOT strip empty strings here, otherwise
 * users cannot remove an existing avatar from the settings UI.
 */
function cleanUpdateOrgRequest<T extends Record<string, unknown>>(input: T): T {
  const output: Record<string, unknown> = {};
  for (const [key, value] of Object.entries(input)) {
    if (value === undefined) continue;
    output[key] = value;
  }
  return output as T;
}

export function useCreateOrg() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (body: CreateOrgRequest): Promise<OrgResponse> => {
      return api.post<OrgResponse>("/orgs", cleanCreateOrgRequest(body));
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: orgsQueryKeys.list() });
    },
  });
}

interface UpdateOrgParams {
  readonly orgId: string;
  readonly body: UpdateOrgRequest;
}

export function useUpdateOrg() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async ({
      orgId,
      body,
    }: UpdateOrgParams): Promise<OrgResponse> => {
      return api.patch<OrgResponse>(
        `/orgs/${orgId}`,
        cleanUpdateOrgRequest(body),
      );
    },
    onSuccess: (_data, variables) => {
      void queryClient.invalidateQueries({ queryKey: orgsQueryKeys.list() });
      void queryClient.invalidateQueries({
        queryKey: orgsQueryKeys.detail(variables.orgId),
      });
    },
  });
}

export function useDeleteOrg() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (orgId: string): Promise<void> => {
      return api.delete<void>(`/orgs/${orgId}`);
    },
    onSuccess: (_data, orgId) => {
      void queryClient.invalidateQueries({ queryKey: orgsQueryKeys.list() });
      void queryClient.removeQueries({
        queryKey: orgsQueryKeys.detail(orgId),
      });
      // Org-inherited user services may disappear.
      void queryClient.invalidateQueries({ queryKey: ["user-services"] });
      void queryClient.invalidateQueries({ queryKey: ["keys"] });
    },
  });
}

export function useSetPrimaryOrg() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (
      body: SetPrimaryOrgRequest,
    ): Promise<{ primary_org_id: string | null }> => {
      return api.patch<{ primary_org_id: string | null }>(
        "/users/me/primary-org",
        body,
      );
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["user"] });
      void queryClient.invalidateQueries({ queryKey: orgsQueryKeys.all });
    },
  });
}

/**
 * Redeem an invite nonce to join the org. Returns the org id and role that
 * the caller was granted. Used by the `/orgs/join/$nonce` redemption page.
 */
export function useRedeemInvite() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (nonce: string): Promise<RedeemInviteResponse> => {
      return api.post<RedeemInviteResponse>(`/orgs/join/${nonce}`);
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: orgsQueryKeys.list() });
      void queryClient.invalidateQueries({ queryKey: ["user-services"] });
      void queryClient.invalidateQueries({ queryKey: ["keys"] });
    },
  });
}
