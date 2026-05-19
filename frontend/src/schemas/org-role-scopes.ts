import { z } from "zod";
import { orgRoleSchema } from "./orgs";

export const orgRoleScopeSchema = z.object({
  role: orgRoleSchema,
  allowed_service_ids: z.array(z.string()).nullable(),
  is_default: z.boolean(),
  updated_at: z.string().nullable(),
  updated_by: z.string().nullable(),
});
export type OrgRoleScope = z.infer<typeof orgRoleScopeSchema>;

export const orgRoleScopesResponseSchema = z.object({
  role_scopes: z.array(orgRoleScopeSchema),
});
export type OrgRoleScopesResponse = z.infer<typeof orgRoleScopesResponseSchema>;

export const updateRoleScopeRequestSchema = z.object({
  allowed_service_ids: z.array(z.string()).nullable(),
});
export type UpdateRoleScopeRequest = z.infer<
  typeof updateRoleScopeRequestSchema
>;
