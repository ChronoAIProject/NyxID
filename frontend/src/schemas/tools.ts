import { z } from "zod";
export const offeringKindSchema = z.enum(["ai_service", "tool"]);
export const publicationSchema = z.enum([
  "draft",
  "validated",
  "published",
  "paused",
]);
export const dataScopeSchema = z.enum(["public", "account", "owned_resource"]);
export const costClassSchema = z.enum(["free", "metered", "resource_backed"]);
export const executionSchema = z.enum([
  "http_operation",
  "job_start",
  "job_poll",
]);
export const importSourceSchema = z.object({
  kind: z.enum(["monid", "vendor_spec", "manual", "catalog_twin"]),
  reference: z.string().max(512),
  version: z.string().max(128).nullable().optional(),
  imported_at: z.string().nullable().optional(),
});
export const toolMetadataFields = {
  offering_kind: offeringKindSchema.optional(),
  topics: z
    .array(z.string().regex(/^[a-z][a-z-]{1,39}$/))
    .max(20)
    .refine((v) => new Set(v).size === v.length, "Topics must be unique")
    .optional(),
  supplier: z.string().max(128).optional(),
};
export const toolCapabilitySchema = z.object({
  supports_proxy_read: z.boolean(),
  supports_proxy_write: z.boolean(),
  supports_proxy_binary_upload: z.boolean(),
  supports_direct_downstream_auth: z.boolean(),
  supports_authoring_via_nyx: z.boolean(),
  supports_websocket: z.boolean(),
  supports_streaming: z.boolean(),
});
export const toolEditableMetadataSchema = z.object({
  ...toolMetadataFields,
  name: z.string().min(1).max(200),
  description: z.string().max(500),
  openapi_spec_url: z.url().or(z.literal("")),
  homepage_url: z.url().or(z.literal("")),
  repository_url: z.url().or(z.literal("")),
  issues_url: z.url().or(z.literal("")),
  examples_url: z.url().or(z.literal("")),
  auth_notes: z.string().max(4096),
  known_limitations: z.string().max(4096),
  capabilities: toolCapabilitySchema,
  required_permissions: z.string().refine((value) => {
    const entries = value
      .split(",")
      .map((entry) => entry.trim())
      .filter(Boolean);
    return (
      entries.length <= 100 && entries.every((entry) => entry.length <= 256)
    );
  }, "Use at most 100 comma-separated permissions, each up to 256 characters"),
});
export const toolManagerDestinationSchema = z.object({
  base_url: z.url().max(2048),
});
export const toolManagerCredentialSchema = z.object({
  credential: z.string().min(1).max(16384),
});
export const addToolSchema = z
  .object({
    ...toolMetadataFields,
    creation_mode: z.enum(["twin", "new"]),
    twin_of_service_id: z.string(),
    name: z.string().min(1).max(200),
    slug: z.string().regex(/^[a-z][a-z0-9-]*$/),
    base_url: z.url().optional().or(z.literal("")),
    auth_method: z.enum(["none", "bearer", "header"]),
    auth_key_name: z.string().max(128),
    openapi_spec_url: z.url().optional().or(z.literal("")),
  })
  .superRefine((value, ctx) => {
    if (value.creation_mode === "twin" && !value.twin_of_service_id)
      ctx.addIssue({
        code: "custom",
        path: ["twin_of_service_id"],
        message: "Choose a source service",
      });
    if (value.creation_mode === "new" && !value.base_url)
      ctx.addIssue({
        code: "custom",
        path: ["base_url"],
        message: "Base URL is required",
      });
  });
export type PublicationState = z.infer<typeof publicationSchema>;
export type ToolLanePrice = {
  metric: string;
  credits_per_unit: string;
  operations?: {
    operation: string;
    label?: string;
    credits_per_unit: string;
    sync_status?: string;
  }[];
};
export type ToolOffering = {
  readonly icon_url?: string | null;
  id: string;
  slug: string;
  name: string;
  description: string | null;
  supplier: string | null;
  topics: string[];
  homepage_url: string | null;
  provider_label: string;
  offering_kind: "tool";
  access: { platform: boolean; byok: boolean };
  pricing: {
    platform: "free" | ToolLanePrice;
    byok: "free" | ToolLanePrice | null;
  };
  limits: { rate_limit_per_second: number; burst: number } | null;
  credential_configured: boolean;
  operations: {
    name: string;
    description: string | null;
    method: string;
    path: string;
    data_scope: z.infer<typeof dataScopeSchema> | null;
    cost_class: z.infer<typeof costClassSchema> | null;
    execution: z.infer<typeof executionSchema>;
    risk: "read" | "write" | null;
  }[];
};
