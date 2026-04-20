import { z } from "zod";
import { credentialSourceSchema } from "./orgs";
import {
  defaultRequestHeaderSchema,
  defaultRequestHeaderUpdateSchema,
} from "./default-request-headers";

export { credentialSourceSchema } from "./orgs";
export type {
  CredentialSource,
  CredentialSourcePersonal,
  CredentialSourceOrg,
} from "./orgs";
export {
  defaultRequestHeaderSchema,
  defaultRequestHeaderListSchema,
  defaultRequestHeaderUpdateSchema,
} from "./default-request-headers";
export type {
  DefaultRequestHeader,
  DefaultRequestHeaderList,
  DefaultRequestHeaderUpdate,
} from "./default-request-headers";

/**
 * Wire-format response for `GET /api/v1/user-services` entries.
 *
 * Every item carries a `credential_source` tag so the client can group
 * personal credentials vs. org-inherited ones and disable write/proxy
 * actions on items the user may see but cannot modify (viewer role or
 * scope-excluded).
 *
 * Note: we only validate the fields the frontend actually reads. Extra
 * fields from the backend pass through unchanged because z.object is
 * permissive by default on unknown keys in Zod 4 when used without
 * `.strict()`.
 */
export const userServiceResponseSchema = z.object({
  id: z.string(),
  slug: z.string(),
  endpoint_id: z.string(),
  api_key_id: z.string().nullable().optional(),
  auth_method: z.string(),
  auth_key_name: z.string(),
  catalog_service_id: z.string().nullable().optional(),
  node_id: z.string().nullable().optional(),
  node_priority: z.number().int(),
  is_active: z.boolean(),
  identity_propagation_mode: z.string(),
  identity_include_user_id: z.boolean(),
  identity_include_email: z.boolean(),
  identity_include_name: z.boolean(),
  identity_jwt_audience: z.string().nullable().optional(),
  forward_access_token: z.boolean(),
  inject_delegation_token: z.boolean(),
  delegation_token_scope: z.string(),
  custom_user_agent: z.string().nullable().optional(),
  /// NyxID#356: per-user default request headers owned by this user
  /// service. Catalog-level admin defaults are surfaced separately.
  default_request_headers: z.array(defaultRequestHeaderSchema).optional(),
  created_at: z.string(),
  updated_at: z.string(),
  credential_source: credentialSourceSchema,
});

export type UserServiceResponse = z.infer<typeof userServiceResponseSchema>;

/**
 * Partial-update payload for `PUT /api/v1/user-services/{id}` and
 * `PUT /api/v1/keys/{id}`. `default_request_headers` follows the backend
 * tri-state: `undefined` leaves unchanged, `null` clears, array
 * replaces.
 */
export const updateUserServiceRequestSchema = z.object({
  auth_method: z.string().optional(),
  auth_key_name: z.string().optional(),
  node_id: z.string().optional(),
  node_priority: z.number().int().optional(),
  is_active: z.boolean().optional(),
  custom_user_agent: z.string().optional(),
  default_request_headers: defaultRequestHeaderUpdateSchema,
});

export type UpdateUserServiceRequest = z.infer<
  typeof updateUserServiceRequestSchema
>;

export const userServiceListResponseSchema = z.object({
  services: z.array(userServiceResponseSchema),
});

export type UserServiceListResponse = z.infer<
  typeof userServiceListResponseSchema
>;
