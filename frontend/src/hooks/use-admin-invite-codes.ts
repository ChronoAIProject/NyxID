import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import type {
  CreateInviteCodeRequest,
  DeactivateInviteCodeResponse,
  InviteCode,
  InviteCodeListResponse,
  UpdateInviteCodeRequest,
} from "@/types/admin";

const QUERY_KEY = ["admin", "invite-codes"] as const;

/// List all invite codes (admin only).
export function useAdminInviteCodes() {
  return useQuery({
    queryKey: QUERY_KEY,
    queryFn: async (): Promise<InviteCodeListResponse> => {
      return api.get<InviteCodeListResponse>("/admin/invite-codes");
    },
  });
}

/// Create a new invite code. Returns the created InviteCode (including its
/// generated code string and id).
export function useCreateInviteCode() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (body: CreateInviteCodeRequest): Promise<InviteCode> => {
      return api.post<InviteCode>("/admin/invite-codes", body);
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: QUERY_KEY });
    },
  });
}

/// Hard ceiling for a single note PATCH. If the backend or network hangs,
/// aborting here releases the drawer's save-in-progress lock so the admin can
/// retry instead of being stuck staring at a spinner indefinitely.
const UPDATE_INVITE_CODE_TIMEOUT_MS = 10_000;

/// Update mutable fields on an invite code. Today only `note` is mutable.
///
/// The PATCH response is the canonical, enriched record (same shape as a list
/// item), so we splice it directly into the cached list rather than firing a
/// background refetch. This means the drawer reflects the saved value the
/// instant the request resolves — even on flaky networks where a follow-up
/// refetch might fail or be delayed.
export function useUpdateInviteCode() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async ({
      id,
      body,
    }: {
      id: string;
      body: UpdateInviteCodeRequest;
    }): Promise<InviteCode> => {
      const controller = new AbortController();
      const timeoutId = window.setTimeout(() => {
        controller.abort();
      }, UPDATE_INVITE_CODE_TIMEOUT_MS);
      try {
        return await api.patch<InviteCode>(
          `/admin/invite-codes/${id}`,
          body,
          { signal: controller.signal },
        );
      } finally {
        window.clearTimeout(timeoutId);
      }
    },
    onSuccess: (updated) => {
      queryClient.setQueryData<InviteCodeListResponse>(QUERY_KEY, (prev) => {
        if (!prev) return prev;
        return {
          invite_codes: prev.invite_codes.map((ic) =>
            ic.id === updated.id ? updated : ic,
          ),
        };
      });
    },
  });
}

/// Deactivate an invite code by ID. Irreversible — admins should mint a new
/// code if a user needs another attempt.
export function useDeactivateInviteCode() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (id: string): Promise<DeactivateInviteCodeResponse> => {
      return api.delete<DeactivateInviteCodeResponse>(
        `/admin/invite-codes/${id}`,
      );
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: QUERY_KEY });
    },
  });
}
