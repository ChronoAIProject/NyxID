import { z } from "zod";
import { findSecretPath } from "./assistant-actions";

const CREDENTIAL_SHAPE =
  /\b\d{6,}:[A-Za-z0-9_-]{20,}|\bsk-(?:proj-|ant-)?[A-Za-z0-9_-]{16,}|\b(?:gh[pousr]_|github_pat_)[A-Za-z0-9_]{16,}|\bxox[baprs]-[A-Za-z0-9-]{12,}|\bAIza[A-Za-z0-9_-]{20,}|-----BEGIN [^-]*PRIVATE KEY-----|\beyJ[A-Za-z0-9_-]+\.eyJ[A-Za-z0-9_-]+\.[A-Za-z0-9_-]+|\bAKIA[A-Z0-9]{16}|\b(?:ornn_|phc_|ya29\.)[A-Za-z0-9_.-]+/;

/** A proposed user message, never an execution or permission grant. */
export const nyxBotHandoffSchema = z
  .object({
    version: z.literal(1),
    title: z.string().trim().min(1).max(120),
    task: z.string().trim().min(1).max(4000),
  })
  .strict()
  .refine(
    (value) =>
      findSecretPath(value) === null &&
      !CREDENTIAL_SHAPE.test(`${value.title}\n${value.task}`),
    {
      message: "Handoffs must not contain credentials.",
    },
  );

export type NyxBotHandoff = z.infer<typeof nyxBotHandoffSchema>;

export function parseNyxBotHandoff(text: string): NyxBotHandoff | null {
  if (text.length > 8192) return null;
  try {
    const result = nyxBotHandoffSchema.safeParse(JSON.parse(text));
    return result.success ? result.data : null;
  } catch {
    return null;
  }
}
