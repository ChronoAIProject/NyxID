import {
  keepPreviousData,
  useMutation,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import type {
  AdminUser,
  AdminUserListResponse,
  AdminSessionListResponse,
  UpdateUserRequest,
  AdminActionResponse,
  RoleUpdateResponse,
  StatusUpdateResponse,
  VerifyEmailResponse,
  RevokeSessionsResponse,
  CreateUserRequest,
  CreateUserResponse,
  AdminAuditLogListParams,
  AdminAuditLogListResponse,
} from "@/types/admin";
import type { PlatformRole } from "@/types/api";

export function useAdminUsers(
  page: number,
  perPage: number,
  search?: string,
  userType?: "person" | "org",
  options?: { readonly enabled?: boolean },
) {
  return useQuery({
    queryKey: ["admin", "users", page, perPage, search, userType],
    queryFn: async (): Promise<AdminUserListResponse> => {
      const params = new URLSearchParams({
        page: String(page),
        per_page: String(perPage),
      });
      if (search) params.set("search", search);
      if (userType) params.set("user_type", userType);
      return api.get<AdminUserListResponse>(
        `/admin/users?${params.toString()}`,
      );
    },
    enabled: options?.enabled ?? true,
  });
}

export function useAdminUser(userId: string) {
  return useQuery({
    queryKey: ["admin", "users", userId],
    queryFn: async (): Promise<AdminUser> => {
      return api.get<AdminUser>(`/admin/users/${userId}`);
    },
    enabled: userId.length > 0,
  });
}

export function useAdminUserSessions(userId: string) {
  return useQuery({
    queryKey: ["admin", "users", userId, "sessions"],
    queryFn: async (): Promise<AdminSessionListResponse> => {
      return api.get<AdminSessionListResponse>(
        `/admin/users/${userId}/sessions`,
      );
    },
    enabled: userId.length > 0,
  });
}

export function useAdminAuditLog(params: AdminAuditLogListParams) {
  return useQuery({
    queryKey: ["admin", "audit-log", params],
    queryFn: async (): Promise<AdminAuditLogListResponse> => {
      const query = new URLSearchParams({
        page: String(params.page),
        per_page: String(params.per_page),
        sort: params.sort,
      });
      if (params.search) query.set("search", params.search);
      if (params.search_filters) {
        query.set("search_filters", params.search_filters);
      }
      if (params.custom_filters) {
        query.set("custom_filters", params.custom_filters);
      }
      if (params.event_type) query.set("event_type", params.event_type);
      if (params.status) query.set("status", params.status);
      if (params.actor) query.set("actor", params.actor);
      if (params.created_dates) {
        query.set("created_dates", params.created_dates);
      }
      if (params.created_from) query.set("created_from", params.created_from);
      if (params.created_to) query.set("created_to", params.created_to);
      return api.get<AdminAuditLogListResponse>(
        `/admin/audit-log?${query.toString()}`,
      );
    },
    placeholderData: keepPreviousData,
  });
}

export function useCreateUser() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (
      data: CreateUserRequest,
    ): Promise<CreateUserResponse> => {
      return api.post<CreateUserResponse>("/admin/users", data);
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["admin", "users"] });
    },
  });
}

export function useUpdateAdminUser() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async ({
      userId,
      data,
    }: {
      readonly userId: string;
      readonly data: UpdateUserRequest;
    }): Promise<AdminUser> => {
      return api.put<AdminUser>(`/admin/users/${userId}`, data);
    },
    onSuccess: (_, { userId }) => {
      void queryClient.invalidateQueries({ queryKey: ["admin", "users"] });
      void queryClient.invalidateQueries({
        queryKey: ["admin", "users", userId],
      });
    },
  });
}

export function useSetUserRole() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async ({
      userId,
      role,
    }: {
      readonly userId: string;
      readonly role: PlatformRole;
    }): Promise<RoleUpdateResponse> => {
      return api.patch<RoleUpdateResponse>(`/admin/users/${userId}/role`, {
        role,
      });
    },
    onSuccess: (_, { userId }) => {
      void queryClient.invalidateQueries({ queryKey: ["admin", "users"] });
      void queryClient.invalidateQueries({
        queryKey: ["admin", "users", userId],
      });
    },
  });
}

export function useSetUserStatus() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async ({
      userId,
      isActive,
    }: {
      readonly userId: string;
      readonly isActive: boolean;
    }): Promise<StatusUpdateResponse> => {
      return api.patch<StatusUpdateResponse>(`/admin/users/${userId}/status`, {
        is_active: isActive,
      });
    },
    onSuccess: (_, { userId }) => {
      void queryClient.invalidateQueries({ queryKey: ["admin", "users"] });
      void queryClient.invalidateQueries({
        queryKey: ["admin", "users", userId],
      });
    },
  });
}

export function useForcePasswordReset() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (userId: string): Promise<AdminActionResponse> => {
      return api.post<AdminActionResponse>(
        `/admin/users/${userId}/reset-password`,
      );
    },
    onSuccess: (_data, userId) => {
      void queryClient.invalidateQueries({ queryKey: ["admin", "users"] });
      void queryClient.invalidateQueries({
        queryKey: ["admin", "users", userId, "sessions"],
      });
    },
  });
}

export function useDeleteUser() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (userId: string): Promise<AdminActionResponse> => {
      return api.delete<AdminActionResponse>(`/admin/users/${userId}`);
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["admin", "users"] });
    },
  });
}

export function useVerifyUserEmail() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (userId: string): Promise<VerifyEmailResponse> => {
      return api.patch<VerifyEmailResponse>(
        `/admin/users/${userId}/verify-email`,
      );
    },
    onSuccess: (_data, userId) => {
      void queryClient.invalidateQueries({ queryKey: ["admin", "users"] });
      void queryClient.invalidateQueries({
        queryKey: ["admin", "users", userId],
      });
    },
  });
}

export function useRevokeUserSessions() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (userId: string): Promise<RevokeSessionsResponse> => {
      return api.delete<RevokeSessionsResponse>(
        `/admin/users/${userId}/sessions`,
      );
    },
    onSuccess: (_data, userId) => {
      void queryClient.invalidateQueries({ queryKey: ["admin", "users"] });
      void queryClient.invalidateQueries({
        queryKey: ["admin", "users", userId, "sessions"],
      });
    },
  });
}
