import { z } from "zod";
export const SAVED_LOGIN_RESIDUAL =
  "A website that deliberately writes a password back into its page as ordinary text could make it visible on screen; use owner takeover for your most sensitive accounts.";
export const exactHttpsOrigin = z.string().refine((value) => {
  try {
    const url = new URL(value);
    return (
      url.protocol === "https:" &&
      url.origin === value &&
      !url.username &&
      !url.password &&
      !url.hostname.includes("*")
    );
  } catch {
    return false;
  }
}, "Use an exact HTTPS origin, such as https://github.com, with no path or wildcard");
export const savedLoginSchema = z.object({
  label: z.string().trim().min(1).max(100),
  allowed_origins: z.array(exactHttpsOrigin).min(1).max(16),
  username: z.string().min(1).max(1024),
  password: z.string().max(16384),
  totp_secret: z.string().max(4096),
  confirm_each_sign_in: z.boolean(),
});
export type SavedLoginInput = z.infer<typeof savedLoginSchema>;
export interface SavedLogin {
  id: string;
  owner_id: string;
  label: string;
  allowed_origins: string[];
  username_hint: string;
  has_password: boolean;
  has_totp: boolean;
  confirm_each_sign_in: boolean;
  created_at: string;
  updated_at: string;
  last_used_at: string | null;
}
