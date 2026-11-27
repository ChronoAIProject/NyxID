import { z } from "zod";
import {
  isValidAvatar,
  readableAvatar,
  type AvatarConfig,
} from "@/lib/assistant/agent-avatar-model";
export const agentAvatarSchema = z
  .object({
    template: z.string(),
    choices: z.record(z.string(), z.string()),
  })
  .refine(readableAvatar);
export const agentAvatarFieldSchema = agentAvatarSchema
  .nullable()
  .optional()
  .catch(null);
export const editableAvatarSchema = agentAvatarSchema.refine(
  (value): value is AvatarConfig => isValidAvatar(value),
  { message: "Choose compatible avatar settings from the face catalogue" },
);
export const agentAvatarFormSchema = z.object({ avatar: editableAvatarSchema });
export type AgentAvatarForm = z.infer<typeof agentAvatarFormSchema>;
export { avatarPatch } from "../../../shared/agent-avatars/access";
