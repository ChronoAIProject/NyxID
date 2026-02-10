import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
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
} from "@/types/admin";

export function useAdminUsers(page: number, perPage: number, search?: string) {
  return useQuery({
    queryKey: ["admin", "users", page, perPage, search],
    queryFn: async (): Promise<AdminUserListResponse> => {
      const params = new URLSearchParams({
        page: String(page),
        per_page: String(perPage),
      });
      if (search) params.set("search", search);
      return api.get<AdminUserListResponse>(`/admin/users?${params.toString()}`);
    },
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
      isAdmin,
    }: {
      readonly userId: string;
      readonly isAdmin: boolean;
    }): Promise<RoleUpdateResponse> => {
      return api.patch<RoleUpdateResponse>(`/admin/users/${userId}/role`, {
        is_admin: isAdmin,
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
    mutationFn: async (
      userId: string,
    ): Promise<AdminActionResponse> => {
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
