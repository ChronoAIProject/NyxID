import { z } from "zod";

export const updateUserSchema = z.object({
  display_name: z
    .string()
    .max(200, "Display name must be 200 characters or less")
    .optional()
    .or(z.literal("")),
  email: z.string().email("Invalid email address").optional().or(z.literal("")),
  avatar_url: z
    .string()
    .url("Must be a valid URL")
    .max(2048, "URL must be 2048 characters or less")
    .optional()
    .or(z.literal("")),
});

export type UpdateUserFormData = z.infer<typeof updateUserSchema>;

export const createUserSchema = z.object({
  email: z.string().min(1, "Email is required").email("Invalid email address"),
  password: z
    .string()
    .min(8, "Password must be at least 8 characters")
    .max(128, "Password must be at most 128 characters"),
  display_name: z
    .string()
    .max(200, "Display name must be 200 characters or less")
    .optional()
    .or(z.literal("")),
  role: z.enum(["admin", "operator", "user"], {
    error: "Role is required",
  }),
});

export type CreateUserFormData = z.infer<typeof createUserSchema>;

// ── Invite codes ──

/// Mirrors the backend `CreateInviteCodeRequest` validator bounds:
/// `max_uses ∈ 1..=1000` and `note ≤ 512` characters. The form keeps
/// `max_uses` as a number (default 10) and `note` as an empty-string-friendly
/// optional field.
export const createInviteCodeSchema = z.object({
  max_uses: z
    .number({ error: "Max uses must be a number" })
    .int("Max uses must be a whole number")
    .min(1, "Max uses must be at least 1")
    .max(1000, "Max uses must be at most 1000"),
  note: z
    .string()
    .max(512, "Note must be 512 characters or less")
    .optional()
    .or(z.literal("")),
});

export type CreateInviteCodeFormData = z.infer<typeof createInviteCodeSchema>;
