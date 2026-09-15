import { z } from "zod";

export const validatorSelectionSchema = z.discriminatedUnion("kind", [
  z.object({ kind: z.literal("stored_only") }),
  z.object({ kind: z.literal("profile"), id: z.string().min(1) }),
]);

export const serviceRequirementSchema = z
  .object({
    id: z
      .string()
      .regex(
        /^[a-z0-9_-]{1,32}$/,
        "Use 1–32 lowercase letters, numbers, underscores or hyphens",
      ),
    label: z.string().trim().min(1).max(160),
    any_of_catalog_slugs: z.array(z.string().min(1)).max(25),
    any_of_catalog_prefix: z.string().max(128).nullable(),
    owner_policy: z.enum(["personal_only", "personal_or_org_allowed"]),
    accepted_credential_types: z.array(z.string()),
    allow_master_credential: z.boolean(),
    allow_no_credential: z.boolean(),
    required_downstream_scopes: z.array(z.string()),
    validator: validatorSelectionSchema,
    optional: z.boolean(),
  })
  .refine(
    (r) => r.any_of_catalog_slugs.length > 0 || !!r.any_of_catalog_prefix,
    {
      message: "Choose at least one catalog service or enter a prefix",
      path: ["any_of_catalog_slugs"],
    },
  );

export const publishManifestSchema = z
  .object({
    enforcement: z.enum(["advise", "gate"]),
    requirements: z.array(serviceRequirementSchema).max(25),
  })
  .refine(
    (m) =>
      new Set(m.requirements.map((r) => r.id)).size === m.requirements.length,
    {
      message: "Requirement IDs must be unique",
      path: ["requirements"],
    },
  );

export const manifestSchema = z.object({
  id: z.string(),
  oauth_client_id: z.string(),
  version: z.number().int().positive(),
  enforcement: z.enum(["advise", "gate"]),
  requirements: z.array(serviceRequirementSchema),
  compiled: z.object({
    catalog_service_ids: z.record(z.string(), z.string()),
    validator_versions: z.record(z.string(), z.number()),
    validators_by_requirement: z.record(z.string(), z.record(z.string(), z.string())),
  }),
  published_by: z.string(),
  published_at: z.string(),
});

export const manifestsResponseSchema = z.object({
  versions: z.array(manifestSchema),
  validator_profiles: z.array(
    z.object({
      id: z.string(),
      version: z.number(),
      claim: z.string(),
      catalog_slugs: z.array(z.string()),
    }),
  ),
});

export const rolloutSchema = z.object({
  effective: z.enum(["disabled", "allowlist", "public"]),
  env_default: z.enum(["disabled", "allowlist", "public"]),
  override_value: z.enum(["disabled", "allowlist", "public"]).nullable(),
  allowed_org_ids: z.array(z.string()),
});

export type PublishManifest = z.infer<typeof publishManifestSchema>;
export type ServiceRequirement = z.infer<typeof serviceRequirementSchema>;
