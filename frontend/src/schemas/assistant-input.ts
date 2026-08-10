import { z } from "zod";
import { actionControlIdentitySchema } from "@/schemas/assistant-actions";

const INPUT_MAX_CHARS = 32_768;
const INPUT_MAX_OPTIONS = 6;

export const inputAnswerSchema = z.union([
  z
    .object({
      freeText: z
        .string()
        .max(INPUT_MAX_CHARS)
        .transform((value) => value.trim())
        .pipe(z.string().min(1)),
    })
    .strict(),
  z
    .object({
      selectedOptionIds: z
        .array(actionControlIdentitySchema)
        .min(1)
        .max(INPUT_MAX_OPTIONS)
        .transform((ids, context) => {
          const normalized = ids.map((id) => id.trim());
          if (new Set(normalized).size !== normalized.length) {
            context.addIssue({
              code: "custom",
              message: "Selected option ids must be distinct",
            });
            return z.NEVER;
          }
          return normalized;
        }),
    })
    .strict(),
]);

export const inputResolveBodySchema = z
  .object({
    type: z.literal("input.resolve"),
    conversationId: actionControlIdentitySchema,
    clientRequestId: actionControlIdentitySchema,
    requestId: actionControlIdentitySchema,
    answer: inputAnswerSchema,
    expectedStateVersion: z.number().int().safe().positive(),
  })
  .strict();

const inputOptionSchema = z
  .object({
    optionId: actionControlIdentitySchema,
    label: z.string().max(4_096),
    description: z.string().max(4_096).optional(),
  })
  .strict();

export const assistantInputRequestSchema = z
  .object({
    requestId: actionControlIdentitySchema,
    prompt: z.string().min(1).max(INPUT_MAX_CHARS),
    options: z.array(inputOptionSchema).max(INPUT_MAX_OPTIONS).default([]),
    allowFreeText: z.boolean().default(false),
    multiSelect: z.boolean().default(false),
  })
  .passthrough()
  .superRefine((request, context) => {
    const optionIds = request.options.map((option) => option.optionId);
    if (new Set(optionIds).size !== optionIds.length) {
      context.addIssue({
        code: "custom",
        path: ["options"],
        message: "Input option ids must be distinct",
      });
    }
    if (!request.allowFreeText && request.options.length === 0) {
      context.addIssue({
        code: "custom",
        path: ["options"],
        message: "Input request has no answer mode",
      });
    }
    if (request.options.length === 1) {
      context.addIssue({
        code: "custom",
        path: ["options"],
        message: "Choice input requires at least two options",
      });
    }
    if (request.options.length === 0 && request.multiSelect) {
      context.addIssue({
        code: "custom",
        path: ["multiSelect"],
        message: "Free-text-only input cannot be multi-select",
      });
    }
  });

export type InputAnswer = z.infer<typeof inputAnswerSchema>;
export type InputResolveBody = z.infer<typeof inputResolveBodySchema>;
export type AssistantInputRequest = z.infer<typeof assistantInputRequestSchema>;

export function buildInputResolveBody(
  conversationId: string,
  clientRequestId: string,
  requestId: string,
  answer: InputAnswer,
  expectedStateVersion: number,
): InputResolveBody {
  return inputResolveBodySchema.parse({
    type: "input.resolve",
    conversationId,
    clientRequestId,
    requestId,
    answer,
    expectedStateVersion,
  });
}
