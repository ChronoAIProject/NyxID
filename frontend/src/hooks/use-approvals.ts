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
      return api.delete<TelegramDisconnectResponse>(
        "/notifications/telegram",
      );
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

export function useApprovalGrants(page: number = 1, perPage: number = 20) {
  return useQuery({
    queryKey: ["approvals", "grants", page, perPage],
    queryFn: async (): Promise<ApprovalGrantListResponse> => {
      const params = new URLSearchParams({
        page: String(page),
        per_page: String(perPage),
      });
      return api.get<ApprovalGrantListResponse>(
        `/approvals/grants?${params.toString()}`,
      );
    },
  });
}

export function useRevokeGrant() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (grantId: string): Promise<RevokeGrantResponse> => {
      return api.delete<RevokeGrantResponse>(
        `/approvals/grants/${grantId}`,
      );
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({
        queryKey: ["approvals", "grants"],
      });
    },
  });
}

// --- Per-Service Approval Configs ---

export function useServiceApprovalConfigs() {
  return useQuery({
    queryKey: ["approvals", "service-configs"],
    queryFn: async (): Promise<ServiceApprovalConfigsResponse> => {
      return api.get<ServiceApprovalConfigsResponse>(
        "/approvals/service-configs",
      );
    },
  });
}

export function useSetServiceApprovalConfig() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async ({
      serviceId,
      approvalRequired,
    }: {
      readonly serviceId: string;
      readonly approvalRequired: boolean;
    }): Promise<SetServiceApprovalConfigResponse> => {
      return api.put<SetServiceApprovalConfigResponse>(
        `/approvals/service-configs/${serviceId}`,
        { approval_required: approvalRequired },
      );
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({
        queryKey: ["approvals", "service-configs"],
      });
    },
  });
}

export function useDeleteServiceApprovalConfig() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (
      serviceId: string,
    ): Promise<DeleteServiceApprovalConfigResponse> => {
      return api.delete<DeleteServiceApprovalConfigResponse>(
        `/approvals/service-configs/${serviceId}`,
      );
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({
        queryKey: ["approvals", "service-configs"],
      });
    },
  });
}
