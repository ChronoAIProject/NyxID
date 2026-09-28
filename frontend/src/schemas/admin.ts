import { z } from "zod";
import { isValidHttpUrl } from "./http-url";

export const updateUserSchema = z.object({
  display_name: z
    .string()
    .max(200, "Display name must be 200 characters or less")
    .optional()
    .or(z.literal("")),
  email: z.string().email("Invalid email address").optional().or(z.literal("")),
  avatar_url: z
    .string()
    .refine(isValidHttpUrl, "Must be a valid URL")
    .refine(
      (v) => v.startsWith("https://"),
      "Avatar URL must use https:// (admin path rejects http)",
    )
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
