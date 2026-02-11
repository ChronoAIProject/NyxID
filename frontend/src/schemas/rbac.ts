import { z } from "zod";

export const createRoleSchema = z.object({
  name: z
    .string()
    .min(1, "Name is required")
    .max(100, "Name must be 100 characters or less"),
  slug: z
    .string()
    .min(1, "Slug is required")
    .max(100, "Slug must be 100 characters or less")
    .regex(/^[a-z0-9_-]+$/, "Slug must be lowercase alphanumeric with hyphens or underscores"),
  description: z
    .string()
    .max(500, "Description must be 500 characters or less")
    .optional()
    .or(z.literal("")),
  permissions: z.string().optional().or(z.literal("")),
  is_default: z.boolean(),
  client_id: z.string().optional().or(z.literal("")),
});

export type CreateRoleFormData = z.infer<typeof createRoleSchema>;

export const updateRoleSchema = z.object({
  name: z
    .string()
    .min(1, "Name is required")
    .max(100, "Name must be 100 characters or less"),
  slug: z
    .string()
    .min(1, "Slug is required")
    .max(100, "Slug must be 100 characters or less")
    .regex(/^[a-z0-9_-]+$/, "Slug must be lowercase alphanumeric with hyphens or underscores"),
  description: z
    .string()
    .max(500, "Description must be 500 characters or less")
    .optional()
    .or(z.literal("")),
  permissions: z.string().optional().or(z.literal("")),
  is_default: z.boolean(),
});

export type UpdateRoleFormData = z.infer<typeof updateRoleSchema>;

export const createGroupSchema = z.object({
  name: z
    .string()
    .min(1, "Name is required")
    .max(100, "Name must be 100 characters or less"),
  slug: z
    .string()
    .min(1, "Slug is required")
    .max(100, "Slug must be 100 characters or less")
    .regex(/^[a-z0-9_-]+$/, "Slug must be lowercase alphanumeric with hyphens or underscores"),
  description: z
    .string()
    .max(500, "Description must be 500 characters or less")
    .optional()
    .or(z.literal("")),
  role_ids: z.string().optional().or(z.literal("")),
  parent_group_id: z.string().optional().or(z.literal("")),
});

export type CreateGroupFormData = z.infer<typeof createGroupSchema>;

export const updateGroupSchema = z.object({
  name: z
    .string()
    .min(1, "Name is required")
    .max(100, "Name must be 100 characters or less"),
  slug: z
    .string()
    .min(1, "Slug is required")
    .max(100, "Slug must be 100 characters or less")
    .regex(/^[a-z0-9_-]+$/, "Slug must be lowercase alphanumeric with hyphens or underscores"),
  description: z
    .string()
    .max(500, "Description must be 500 characters or less")
    .optional()
    .or(z.literal("")),
  role_ids: z.string().optional().or(z.literal("")),
  parent_group_id: z.string().optional().or(z.literal("")),
});

export type UpdateGroupFormData = z.infer<typeof updateGroupSchema>;
