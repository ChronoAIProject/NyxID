import { useMutation, useQuery, useQueryClient } from "@tanstack/react-query";
import { api } from "@/lib/api-client";
import type {
  Role,
  RoleListResponse,
  Group,
  GroupListResponse,
  GroupMembersResponse,
  UserRolesResponse,
  UserGroupsResponse,
  CreateRoleRequest,
  UpdateRoleRequest,
  CreateGroupRequest,
  UpdateGroupRequest,
  RoleAssignmentResponse,
  GroupMembershipResponse,
} from "@/types/rbac";

// --- Role Hooks ---

export function useRoles() {
  return useQuery({
    queryKey: ["admin", "roles"],
    queryFn: async (): Promise<RoleListResponse> => {
      return api.get<RoleListResponse>("/admin/roles");
    },
  });
}

export function useRole(roleId: string) {
  return useQuery({
    queryKey: ["admin", "roles", roleId],
    queryFn: async (): Promise<Role> => {
      return api.get<Role>(`/admin/roles/${roleId}`);
    },
    enabled: roleId.length > 0,
  });
}

export function useCreateRole() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (data: CreateRoleRequest): Promise<Role> => {
      return api.post<Role>("/admin/roles", data);
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["admin", "roles"] });
    },
  });
}

export function useUpdateRole() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async ({
      roleId,
      data,
    }: {
      readonly roleId: string;
      readonly data: UpdateRoleRequest;
    }): Promise<Role> => {
      return api.put<Role>(`/admin/roles/${roleId}`, data);
    },
    onSuccess: (_, { roleId }) => {
      void queryClient.invalidateQueries({ queryKey: ["admin", "roles"] });
      void queryClient.invalidateQueries({
        queryKey: ["admin", "roles", roleId],
      });
    },
  });
}

export function useDeleteRole() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (roleId: string): Promise<void> => {
      return api.delete<void>(`/admin/roles/${roleId}`);
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["admin", "roles"] });
    },
  });
}

// --- Role Assignment Hooks ---

export function useUserRoles(userId: string) {
  return useQuery({
    queryKey: ["admin", "users", userId, "roles"],
    queryFn: async (): Promise<UserRolesResponse> => {
      return api.get<UserRolesResponse>(`/admin/users/${userId}/roles`);
    },
    enabled: userId.length > 0,
  });
}

export function useAssignRole() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async ({
      userId,
      roleId,
    }: {
      readonly userId: string;
      readonly roleId: string;
    }): Promise<RoleAssignmentResponse> => {
      return api.post<RoleAssignmentResponse>(
        `/admin/users/${userId}/roles/${roleId}`,
      );
    },
    onSuccess: (_, { userId }) => {
      void queryClient.invalidateQueries({
        queryKey: ["admin", "users", userId, "roles"],
      });
      void queryClient.invalidateQueries({
        queryKey: ["admin", "users", userId],
      });
    },
  });
}

export function useRevokeRole() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async ({
      userId,
      roleId,
    }: {
      readonly userId: string;
      readonly roleId: string;
    }): Promise<RoleAssignmentResponse> => {
      return api.delete<RoleAssignmentResponse>(
        `/admin/users/${userId}/roles/${roleId}`,
      );
    },
    onSuccess: (_, { userId }) => {
      void queryClient.invalidateQueries({
        queryKey: ["admin", "users", userId, "roles"],
      });
      void queryClient.invalidateQueries({
        queryKey: ["admin", "users", userId],
      });
    },
  });
}

// --- Group Hooks ---

export function useGroups() {
  return useQuery({
    queryKey: ["admin", "groups"],
    queryFn: async (): Promise<GroupListResponse> => {
      return api.get<GroupListResponse>("/admin/groups");
    },
  });
}

export function useGroup(groupId: string) {
  return useQuery({
    queryKey: ["admin", "groups", groupId],
    queryFn: async (): Promise<Group> => {
      return api.get<Group>(`/admin/groups/${groupId}`);
    },
    enabled: groupId.length > 0,
  });
}

export function useCreateGroup() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (data: CreateGroupRequest): Promise<Group> => {
      return api.post<Group>("/admin/groups", data);
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["admin", "groups"] });
    },
  });
}

export function useUpdateGroup() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async ({
      groupId,
      data,
    }: {
      readonly groupId: string;
      readonly data: UpdateGroupRequest;
    }): Promise<Group> => {
      return api.put<Group>(`/admin/groups/${groupId}`, data);
    },
    onSuccess: (_, { groupId }) => {
      void queryClient.invalidateQueries({ queryKey: ["admin", "groups"] });
      void queryClient.invalidateQueries({
        queryKey: ["admin", "groups", groupId],
      });
    },
  });
}

export function useDeleteGroup() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async (groupId: string): Promise<void> => {
      return api.delete<void>(`/admin/groups/${groupId}`);
    },
    onSuccess: () => {
      void queryClient.invalidateQueries({ queryKey: ["admin", "groups"] });
    },
  });
}

// --- Group Membership Hooks ---

export function useGroupMembers(groupId: string) {
  return useQuery({
    queryKey: ["admin", "groups", groupId, "members"],
    queryFn: async (): Promise<GroupMembersResponse> => {
      return api.get<GroupMembersResponse>(
        `/admin/groups/${groupId}/members`,
      );
    },
    enabled: groupId.length > 0,
  });
}

export function useAddGroupMember() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async ({
      groupId,
      userId,
    }: {
      readonly groupId: string;
      readonly userId: string;
    }): Promise<GroupMembershipResponse> => {
      return api.post<GroupMembershipResponse>(
        `/admin/groups/${groupId}/members/${userId}`,
      );
    },
    onSuccess: (_, { groupId }) => {
      void queryClient.invalidateQueries({
        queryKey: ["admin", "groups", groupId, "members"],
      });
      void queryClient.invalidateQueries({
        queryKey: ["admin", "groups", groupId],
      });
    },
  });
}

export function useRemoveGroupMember() {
  const queryClient = useQueryClient();

  return useMutation({
    mutationFn: async ({
      groupId,
      userId,
    }: {
      readonly groupId: string;
      readonly userId: string;
    }): Promise<GroupMembershipResponse> => {
      return api.delete<GroupMembershipResponse>(
        `/admin/groups/${groupId}/members/${userId}`,
      );
    },
    onSuccess: (_, { groupId }) => {
      void queryClient.invalidateQueries({
        queryKey: ["admin", "groups", groupId, "members"],
      });
      void queryClient.invalidateQueries({
        queryKey: ["admin", "groups", groupId],
      });
    },
  });
}

export function useUserGroups(userId: string) {
  return useQuery({
    queryKey: ["admin", "users", userId, "groups"],
    queryFn: async (): Promise<UserGroupsResponse> => {
      return api.get<UserGroupsResponse>(`/admin/users/${userId}/groups`);
    },
    enabled: userId.length > 0,
  });
}
