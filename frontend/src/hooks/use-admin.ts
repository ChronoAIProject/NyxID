import {
  keepPreviousData,
  useMutation,
  useQuery,
  useQueryClient,
} from "@tanstack/react-query";
import { api, apiClient } from "@/lib/api-client";
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

function auditLogFilterQuery(params: AdminAuditLogListParams): URLSearchParams {
  const query = new URLSearchParams();
  for (const key of [
    "user_id",
    "api_key_id",
    "search",
    "search_filters",
    "custom_filters",
    "event_type",
    "status",
    "actor",
    "created_dates",
    "created_from",
    "created_to",
  ] as const) {
    if (params[key]) query.set(key, params[key]);
  }
  return query;
}

export function useAdminAuditLog(params: AdminAuditLogListParams) {
  const filters = auditLogFilterQuery(params).toString();
  const page = useQuery({
    queryKey: ["admin", "audit-log", "page", params],
    queryFn: async ({ signal }): Promise<AdminAuditLogListResponse> => {
      const query = new URLSearchParams({
        page: String(params.page),
        per_page: String(params.per_page),
        sort: params.sort,
        include_total: "false",
      });
      auditLogFilterQuery(params).forEach((value, key) =>
        query.set(key, value),
      );
      return apiClient<AdminAuditLogListResponse>(
        `/admin/audit-log?${query.toString()}`,
        { signal },
      );
    },
    placeholderData: keepPreviousData,
  });

  const needsTotal =
    page.data !== undefined &&
    page.data.total === undefined &&
    !page.isPlaceholderData;
  const count = useQuery({
    queryKey: ["admin", "audit-log", "count", filters],
    queryFn: ({ signal }) =>
      apiClient<{ readonly total: number }>(`/admin/audit-log/count?${filters}`, {
        signal,
      }),
    enabled: needsTotal,
    staleTime: 60_000,
    retry: false,
  });

  return {
    ...page,
    total:
      page.data?.total ??
      (!page.isPlaceholderData ? count.data?.total : undefined),
    totalError: needsTotal ? count.error : null,
    refetchTotal: count.refetch,
    refetch: () =>
      Promise.all([page.refetch(), ...(needsTotal ? [count.refetch()] : [])]),
  };
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
