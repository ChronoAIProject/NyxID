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
export type ToolOffering = {
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
    platform: "free" | { metric: string; credits_per_unit: string };
    byok: "free" | { metric: string; credits_per_unit: string } | null;
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
