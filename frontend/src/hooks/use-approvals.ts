import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import type {
  NotificationSettings,
  UpdateNotificationSettingsRequest,
  TelegramLinkResponse,
  TelegramDisconnectResponse,
  ApprovalRequestListResponse,
  ApprovalGrantListResponse,
  ApprovalDecideResponse,
  RevokeGrantResponse,
  ServiceApprovalConfigsResponse,
  SetServiceApprovalConfigResponse,
  DeleteServiceApprovalConfigResponse,
  PushDevicesResponse,
  RemoveDeviceResponse,
  ApprovalMode,
} from "@/types/approvals";

// --- Notification Settings ---

export function useNotificationSettings() {
  return useQuery({
    queryKey: ["notifications", "settings"],
    queryFn: async (): Promise<NotificationSettings> => {
      return api.get<NotificationSettings>("/notifications/settings");
    },
  });
}

export function useUpdateNotificationSettings() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (
      data: UpdateNotificationSettingsRequest,
    ): Promise<NotificationSettings> => {
      return api.put<NotificationSettings>("/notifications/settings", data);
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({
        queryKey: ["notifications", "settings"],
      });
    },
  });
}

// --- Telegram Linking ---

export function useTelegramLink() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (): Promise<TelegramLinkResponse> => {
      return api.post<TelegramLinkResponse>("/notifications/telegram/link");
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({
        queryKey: ["notifications", "settings"],
      });
    },
  });
}

export function useTelegramDisconnect() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (): Promise<TelegramDisconnectResponse> => {
      return api.delete<TelegramDisconnectResponse>("/notifications/telegram");
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({
        queryKey: ["notifications", "settings"],
      });
    },
  });
}

// --- Push Devices ---

export function usePushDevices() {
  return useQuery({
    queryKey: ["notifications", "devices"],
    queryFn: async (): Promise<PushDevicesResponse> => {
      return api.get<PushDevicesResponse>("/notifications/devices");
    },
  });
}

export function useRemoveDevice() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (deviceId: string): Promise<RemoveDeviceResponse> => {
      return api.delete<RemoveDeviceResponse>(
        `/notifications/devices/${deviceId}`,
      );
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({
        queryKey: ["notifications", "devices"],
      });
      void queryClient.invalidateQueries({
        queryKey: ["notifications", "settings"],
      });
    },
  });
}

// --- Approval Requests ---

export function useApprovalRequests(
  page: number = 1,
  perPage: number = 20,
  status?: string,
) {
  return useQuery({
    queryKey: ["approvals", "requests", page, perPage, status],
    queryFn: async (): Promise<ApprovalRequestListResponse> => {
      const params = new URLSearchParams({
        page: String(page),
        per_page: String(perPage),
      });
      if (status) params.set("status", status);
      return api.get<ApprovalRequestListResponse>(
        `/approvals/requests?${params.toString()}`,
      );
    },
  });
}

export function useDecideApproval() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async ({
      requestId,
      approved,
    }: {
      readonly requestId: string;
      readonly approved: boolean;
    }): Promise<ApprovalDecideResponse> => {
      return api.post<ApprovalDecideResponse>(
        `/approvals/requests/${requestId}/decide`,
        { approved },
      );
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({
        queryKey: ["approvals", "requests"],
      });
      void queryClient.invalidateQueries({
        queryKey: ["approvals", "grants"],
      });
    },
  });
}

// --- Approval Grants ---

/**
 * List active approval grants. When `orgId` is set, list grants owned by
 * the org instead of the caller's personal scope -- caller must be an
 * admin of that org. Org-policy approvals create grants under the org
 * user_id, so this is the only way for org admins to manage them.
 */
export function useApprovalGrants(
  page: number = 1,
  perPage: number = 20,
  orgId?: string,
) {
  return useQuery({
    queryKey: ["approvals", "grants", page, perPage, orgId ?? "personal"],
    queryFn: async (): Promise<ApprovalGrantListResponse> => {
      const params = new URLSearchParams({
        page: String(page),
        per_page: String(perPage),
      });
      if (orgId) params.set("org_id", orgId);
      return api.get<ApprovalGrantListResponse>(
        `/approvals/grants?${params.toString()}`,
      );
    },
  });
}

export function useRevokeGrant() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (params: {
      readonly grantId: string;
      /** When set, the grant is org-owned and the caller must be an
       *  admin of that org. */
      readonly orgId?: string;
    }): Promise<RevokeGrantResponse> => {
      const { grantId, orgId } = params;
      const path = orgId
        ? `/approvals/grants/${grantId}?org_id=${encodeURIComponent(orgId)}`
        : `/approvals/grants/${grantId}`;
      return api.delete<RevokeGrantResponse>(path);
    },
    onSuccess: () => {
      // Broad invalidate -- the grants query key already varies by
      // [page, perPage, orgId|"personal"], so this nukes both the org
      // and personal lists rather than reasoning about the exact key.
      void queryClient.invalidateQueries({
        queryKey: ["approvals", "grants"],
      });
    },
  });
}

// --- Per-Service Approval Configs ---

/**
 * List per-service approval configs. When `orgId` is set, lists configs
 * scoped to the given org instead of the caller's personal scope -- the
 * caller must be an admin of the org. Used by the org settings UI to
 * show approval policies on org-shared services.
 */
export function useServiceApprovalConfigs(orgId?: string) {
  return useQuery({
    queryKey: ["approvals", "service-configs", orgId ?? "personal"],
    queryFn: async (): Promise<ServiceApprovalConfigsResponse> => {
      const path = orgId
        ? `/approvals/service-configs?org_id=${encodeURIComponent(orgId)}`
        : "/approvals/service-configs";
      return api.get<ServiceApprovalConfigsResponse>(path);
    },
  });
}

export function useSetServiceApprovalConfig() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async ({
      serviceId,
      approvalRequired,
      approvalMode,
      orgId,
    }: {
      readonly serviceId: string;
      readonly approvalRequired?: boolean;
      readonly approvalMode?: ApprovalMode;
      /** When set, the policy is set on the given org's behalf. Caller
       *  must be an admin of that org. The org's policy is dominant for
       *  org-shared services. */
      readonly orgId?: string;
    }): Promise<SetServiceApprovalConfigResponse> => {
      const path = orgId
        ? `/approvals/service-configs/${serviceId}?org_id=${encodeURIComponent(orgId)}`
        : `/approvals/service-configs/${serviceId}`;
      return api.put<SetServiceApprovalConfigResponse>(path, {
        ...(approvalRequired !== undefined
          ? { approval_required: approvalRequired }
          : {}),
        ...(approvalMode !== undefined ? { approval_mode: approvalMode } : {}),
      });
    },
    onSuccess: (_, variables) => {
      void queryClient.invalidateQueries({
        queryKey: ["approvals", "service-configs", variables.orgId ?? "personal"],
      });
    },
  });
}

export function useDeleteServiceApprovalConfig() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async ({
      serviceId,
      orgId,
    }: {
      readonly serviceId: string;
      readonly orgId?: string;
    }): Promise<DeleteServiceApprovalConfigResponse> => {
      const path = orgId
        ? `/approvals/service-configs/${serviceId}?org_id=${encodeURIComponent(orgId)}`
        : `/approvals/service-configs/${serviceId}`;
      return api.delete<DeleteServiceApprovalConfigResponse>(path);
    },
    onSuccess: (_, variables) => {
      void queryClient.invalidateQueries({
        queryKey: ["approvals", "service-configs", variables.orgId ?? "personal"],
      });
    },
  });
}
