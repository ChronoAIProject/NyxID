import { z } from "zod";

// ─────────────────────────────────────────────────────────────────────────────
// Enums
// ─────────────────────────────────────────────────────────────────────────────

/**
 * Org role grants. Mirrors `OrgRole` on the backend.
 * - admin: manage org, members, invites, and shared services
 * - member: use org services via the proxy
 * - viewer: see org services exist but cannot proxy through them
 */
export const ORG_ROLES = ["owner", "admin", "member", "viewer"] as const;

export const orgRoleSchema = z.enum(ORG_ROLES);
export type OrgRole = z.infer<typeof orgRoleSchema>;

export const scopeSourceSchema = z.enum(["inherit", "override"]);
export type ScopeSource = z.infer<typeof scopeSourceSchema>;

// Mirror backend validate_slug() in admin_helpers.rs and the UUID-shape
// rejection in org_slug.rs. 1-80 chars; lowercase a-z, digits, single
// hyphens; cannot start or end with hyphen; cannot be UUID-shaped.
const ORG_SLUG_REGEX = /^[a-z0-9](?:[a-z0-9-]*[a-z0-9])?$/;
const UUID_SHAPE_REGEX =
  /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/;
export const orgSlugSchema = z
  .string()
  .min(1, "Slug is required")
  .max(80, "Slug must be 80 characters or fewer")
  .regex(ORG_SLUG_REGEX, "Use lowercase letters, digits, and single hyphens")
  .refine((v) => !UUID_SHAPE_REGEX.test(v), "Slug must not be UUID-shaped");

// ─────────────────────────────────────────────────────────────────────────────
// Response shapes (match backend wire format exactly)
// ─────────────────────────────────────────────────────────────────────────────

/**
 * A single org summary returned from list endpoints.
 * Note: backend returns snake_case; we do not transform.
 */
export const orgListItemSchema = z.object({
  id: z.string(),
  slug: z.string(),
  display_name: z.string().nullable(),
  avatar_url: z.string().nullable(),
  /**
   * User-visible contact email. `null` when the backend stores the synthetic
   * `org-<uuid>@nyxid.local` placeholder (i.e. the admin did not set one at
   * creation time).
   */
  contact_email: z.string().nullable(),
  your_role: orgRoleSchema,
  created_at: z.string(),
});
export type OrgListItem = z.infer<typeof orgListItemSchema>;

export const orgListResponseSchema = z.object({
  orgs: z.array(orgListItemSchema),
});
export type OrgListResponse = z.infer<typeof orgListResponseSchema>;

/**
 * Single org detail response.
 */
export const orgResponseSchema = z.object({
  id: z.string(),
  slug: z.string(),
  display_name: z.string().nullable(),
  avatar_url: z.string().nullable(),
  /**
   * See `orgListItemSchema.contact_email`.
   */
  contact_email: z.string().nullable(),
  created_at: z.string(),
  remote_credential_integrity_verification_opt_out: z.boolean(),
  your_role: orgRoleSchema,
  member_count: z.number().int().nonnegative(),
});
export type OrgResponse = z.infer<typeof orgResponseSchema>;

/**
 * A member of an org.
 */
export const memberResponseSchema = z.object({
  membership_id: z.string(),
  user_id: z.string(),
  display_name: z.string().nullable(),
  email: z.string().nullable(),
  role: orgRoleSchema,
  scope_source: scopeSourceSchema,
  allowed_service_ids: z.array(z.string()).nullable(),
  effective_allowed_service_ids: z.array(z.string()).nullable(),
  created_at: z.string(),
  revoked_at: z.string().nullable(),
});
export type MemberResponse = z.infer<typeof memberResponseSchema>;

export const memberListResponseSchema = z.object({
  members: z.array(memberResponseSchema),
});
export type MemberListResponse = z.infer<typeof memberListResponseSchema>;

/**
 * A pending or redeemed org invite.
 *
 * `redeemed_by_email` and `redeemed_by_display_name` are populated by the
 * backend for redeemed invites so the admin UI can show who used each
 * invite without an N+1 user lookup (issue #409).
 */
export const inviteResponseSchema = z.object({
  id: z.string(),
  nonce: z.string(),
  role: orgRoleSchema,
  scope_source: scopeSourceSchema,
  allowed_service_ids: z.array(z.string()).nullable(),
  created_by: z.string(),
  expires_at: z.string(),
  redeemed_by: z.string().nullable(),
  redeemed_by_email: z.string().nullable().optional(),
  redeemed_by_display_name: z.string().nullable().optional(),
  redeemed_at: z.string().nullable(),
  created_at: z.string(),
});
export type InviteResponse = z.infer<typeof inviteResponseSchema>;

export const inviteListResponseSchema = z.object({
  invites: z.array(inviteResponseSchema),
});
export type InviteListResponse = z.infer<typeof inviteListResponseSchema>;

/**
 * Response from `POST /orgs/join/{nonce}`.
 */
export const redeemInviteResponseSchema = z.object({
  org_id: z.string(),
  role: orgRoleSchema,
});
export type RedeemInviteResponse = z.infer<typeof redeemInviteResponseSchema>;

// ─────────────────────────────────────────────────────────────────────────────
// Request shapes (used by forms + mutations)
// ─────────────────────────────────────────────────────────────────────────────

export const createOrgRequestSchema = z.object({
  display_name: z
    .string()
    .trim()
    .min(1, "Display name is required")
    .max(128, "Display name must be at most 128 characters"),
  contact_email: z
    .string()
    .trim()
    .email("Contact email must be a valid email")
    .optional()
    .or(z.literal("")),
  avatar_url: z.string().trim().url("Avatar URL must be valid").optional().or(
    z.literal(""),
  ),
});
export type CreateOrgRequest = z.infer<typeof createOrgRequestSchema>;

export const updateOrgRequestSchema = z.object({
  display_name: z
    .string()
    .trim()
    .min(1, "Display name is required")
    .max(128, "Display name must be at most 128 characters")
    .optional(),
  slug: orgSlugSchema.optional(),
  avatar_url: z.string().trim().url("Avatar URL must be valid").optional().or(
    z.literal(""),
  ),
  /**
   * Pass an empty string to clear the contact email back to the synthetic
   * placeholder on the backend. Omit to leave unchanged.
   */
  contact_email: z
    .string()
    .trim()
    .email("Contact email must be a valid email")
    .optional()
    .or(z.literal("")),
  remote_credential_integrity_verification_opt_out: z.boolean().optional(),
});
export type UpdateOrgRequest = z.infer<typeof updateOrgRequestSchema>;

export const addMemberRequestSchema = z.object({
  user_id: z.string().min(1, "User id is required"),
  role: orgRoleSchema,
  scope_source: scopeSourceSchema.optional(),
  allowed_service_ids: z.array(z.string()).optional(),
});
export type AddMemberRequest = z.infer<typeof addMemberRequestSchema>;

export const updateMemberRequestSchema = z.object({
  role: orgRoleSchema.optional(),
  scope_source: scopeSourceSchema.optional(),
  /**
   * Backend uses Option<Option<Vec<String>>>: omit to leave unchanged, `null`
   * to clear (full access), or an array to restrict.
   * On the wire this is represented as either absent, `null`, or an array.
   */
  allowed_service_ids: z.array(z.string()).nullable().optional(),
});
export type UpdateMemberRequest = z.infer<typeof updateMemberRequestSchema>;

export const createInviteRequestSchema = z.object({
  role: orgRoleSchema,
  scope_source: scopeSourceSchema.optional(),
  allowed_service_ids: z.array(z.string()).optional(),
  /**
   * TTL in hours. Defaults server-side to 24 if omitted.
   */
  ttl_hours: z
    .number()
    .int("TTL must be a whole number of hours")
    .positive("TTL must be positive")
    .max(24 * 30, "TTL must be at most 30 days")
    .optional(),
});
export type CreateInviteRequest = z.infer<typeof createInviteRequestSchema>;

export const setPrimaryOrgRequestSchema = z.object({
  primary_org_id: z.string().nullable(),
});
export type SetPrimaryOrgRequest = z.infer<typeof setPrimaryOrgRequestSchema>;

// ─────────────────────────────────────────────────────────────────────────────
// Credential source discriminated union
// ─────────────────────────────────────────────────────────────────────────────

/**
 * Provenance of a user service. Mirrors
 * `CredentialSourceResponse` on the backend.
 *
 * - `personal`: owned directly by the actor
 * - `org`: inherited from an org membership; `allowed = false` for viewer role
 *   or scope-excluded services
 */
export const credentialSourcePersonalSchema = z.object({
  type: z.literal("personal"),
});

export const credentialSourceOrgSchema = z.object({
  type: z.literal("org"),
  org_id: z.string(),
  org_name: z.string(),
  /**
   * Org avatar URL (when configured). The AI Services page uses this so
   * shared org sources render the same avatar shown on the Organizations
   * page (#545). May be omitted by older backends; treat as null.
   */
  avatar_url: z.string().nullish(),
  role: orgRoleSchema,
  allowed: z.boolean(),
});

export const credentialSourceSchema = z.discriminatedUnion("type", [
  credentialSourcePersonalSchema,
  credentialSourceOrgSchema,
]);

export type CredentialSource = z.infer<typeof credentialSourceSchema>;
export type CredentialSourcePersonal = z.infer<
  typeof credentialSourcePersonalSchema
>;
export type CredentialSourceOrg = z.infer<typeof credentialSourceOrgSchema>;
