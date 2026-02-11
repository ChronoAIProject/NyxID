export interface Role {
  readonly id: string;
  readonly name: string;
  readonly slug: string;
  readonly description: string | null;
  readonly permissions: readonly string[];
  readonly is_default: boolean;
  readonly is_system: boolean;
  readonly client_id: string | null;
  readonly created_at: string;
  readonly updated_at: string;
}

export interface RoleListResponse {
  readonly roles: readonly Role[];
}

export interface Group {
  readonly id: string;
  readonly name: string;
  readonly slug: string;
  readonly description: string | null;
  readonly roles: readonly Role[];
  readonly parent_group_id: string | null;
  readonly member_count: number;
  readonly created_at: string;
  readonly updated_at: string;
}

export interface GroupListResponse {
  readonly groups: readonly Group[];
}

export interface GroupMember {
  readonly id: string;
  readonly email: string;
  readonly display_name: string | null;
}

export interface GroupMembersResponse {
  readonly members: readonly GroupMember[];
  readonly total: number;
}

export interface UserRolesResponse {
  readonly direct_roles: readonly Role[];
  readonly inherited_roles: readonly Role[];
  readonly effective_permissions: readonly string[];
}

export interface UserGroupsResponse {
  readonly groups: readonly Group[];
}

export interface Consent {
  readonly id: string;
  readonly client_id: string;
  readonly client_name: string;
  readonly scopes: string;
  readonly granted_at: string;
  readonly expires_at: string | null;
}

export interface ConsentListResponse {
  readonly consents: readonly Consent[];
}

export interface CreateRoleRequest {
  readonly name: string;
  readonly slug: string;
  readonly description?: string;
  readonly permissions: readonly string[];
  readonly is_default?: boolean;
  readonly client_id?: string;
}

export interface UpdateRoleRequest {
  readonly name?: string;
  readonly slug?: string;
  readonly description?: string;
  readonly permissions?: readonly string[];
  readonly is_default?: boolean;
}

export interface CreateGroupRequest {
  readonly name: string;
  readonly slug: string;
  readonly description?: string;
  readonly role_ids: readonly string[];
  readonly parent_group_id?: string;
}

export interface UpdateGroupRequest {
  readonly name?: string;
  readonly slug?: string;
  readonly description?: string;
  readonly role_ids?: readonly string[];
  readonly parent_group_id?: string;
}

export interface RoleAssignmentResponse {
  readonly message: string;
}

export interface GroupMembershipResponse {
  readonly message: string;
}

export interface ConsentRevokeResponse {
  readonly message: string;
}
