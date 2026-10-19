import { z } from "zod";

const uuid = z.uuid();

export const keyReadGrantSchema = z.object({
  user_service_ids: z.string().refine((value) => {
    const ids = value.split(/[,\s]+/).filter(Boolean);
    return ids.length >= 1 && ids.length <= 100 && new Set(ids).size === ids.length && ids.every((id) => uuid.safeParse(id).success);
  }, "Enter 1–100 distinct user-service UUIDs"),
  expires_at: z.string().refine((value) => value === "" || (!Number.isNaN(Date.parse(value)) && Date.parse(value) > Date.now()), "Choose a future expiry"),
});

export type KeyReadGrantFormData = z.infer<typeof keyReadGrantSchema>;
