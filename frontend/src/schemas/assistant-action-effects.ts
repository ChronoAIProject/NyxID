import { z } from "zod";
import { actionControlIdentitySchema } from "@/schemas/assistant-actions";

export const assistantOneTimeMaterialSchema = z
  .enum(["delivered", "unavailable"])
  .optional();

export const assistantServiceAccessReviewRequestSchema = z
  .object({
    actionRequestId: actionControlIdentitySchema,
    userServiceId: actionControlIdentitySchema,
    serviceSlug: z.string().min(1).max(128),
    resourceUri: z.string().min(1).max(512),
  })
  .strict();

export const assistantServiceAccessReviewResponseSchema = z
  .object({
    resource: z.object({ userServiceId: actionControlIdentitySchema }).strict(),
    replayed: z.boolean(),
  })
  .strict();

export type AssistantServiceAccessReviewEffectRequest = z.infer<
  typeof assistantServiceAccessReviewRequestSchema
>;
export type AssistantServiceAccessReviewEffectResponse = z.infer<
  typeof assistantServiceAccessReviewResponseSchema
>;
