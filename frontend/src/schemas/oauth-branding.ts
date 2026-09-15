import { z } from "zod";

export const logoUrlSchema = z
  .string()
  .regex(/^\/api\/v1\/branding\/assets\/[0-9a-f-]{36}$/);
export const brandingSchema = z.object({
  logo_asset_id: z.string().nullable(),
  logo_url: logoUrlSchema.nullable(),
  homepage_url: z.string().nullable(),
  branding_revision: z.number().int().nonnegative(),
  branding_verified_revision: z.number().int().nonnegative().nullable(),
  verified: z.boolean(),
});
export const authorizeContextSchema = z.object({
  client_name: z.string(),
  handoff_blurb: z.string().nullable(),
  logo_url: logoUrlSchema.nullable(),
  homepage_url: z.string().nullable(),
  verified: z.boolean(),
  destination: z.string(),
});
export const homepageFormSchema = z.object({
  homepage_url: z
    .string()
    .trim()
    .max(2048)
    .refine((value) => {
      if (!value) return true;
      try {
        return new URL(value).protocol === "https:";
      } catch {
        return false;
      }
    }, "Use an HTTPS homepage URL"),
});
export type HomepageForm = z.infer<typeof homepageFormSchema>;
