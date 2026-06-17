/**
 * Zod schemas for the CLI wizard text inputs. Values mirror the backend
 * authoritative regexes so invalid input is rejected client-side before
 * the user hits submit — no more "HTTP 400 · Node name must contain only
 * lowercase letters…" round-trip surfacing to the user.
 *
 * Source of truth (backend):
 * - Node name:   `backend/src/services/node_service.rs:27-41`
 *                1-64 chars, `[a-z0-9-]` only
 * - API key name: `backend/src/services/key_service.rs:168-172`
 *                 1-200 chars, no character restriction
 * - Service slug: `backend/src/services/user_service_service.rs:131-152`
 *                 1-64 chars, `[a-z0-9-]`, no leading/trailing hyphen
 * - Platform:    `backend/src/services/key_service.rs:47-62`
 *                 empty string OR one of a fixed list
 *
 * Backend remains the authoritative validator; these schemas exist only
 * to catch the common cases client-side for a better UX. Any server-only
 * rules (uniqueness, ownership checks, org scope filtering) still surface
 * via the existing error-banner path.
 */

import { z } from "zod"

/** Slug / kebab-case identifier constraint used by nodes and services. */
const KEBAB_CASE = /^[a-z0-9-]+$/

export const nodeNameSchema = z
  .string()
  .min(1, "Node name is required")
  .max(64, "Node name must be 64 characters or fewer")
  .regex(KEBAB_CASE, "Lowercase letters, digits, and hyphens only")

/**
 * Slug validator: kebab-case with no leading or trailing hyphen.
 * Applies to the auto-derived slug on `service add` and anywhere a
 * user can pick a service identifier manually.
 */
export const serviceSlugSchema = z
  .string()
  .min(1, "Slug is required")
  .max(64, "Slug must be 64 characters or fewer")
  .regex(KEBAB_CASE, "Lowercase letters, digits, and hyphens only")
  .refine((v) => !v.startsWith("-") && !v.endsWith("-"), {
    message: "Slug must not start or end with a hyphen",
  })

/**
 * API key name: the backend only caps length, so the client rules are
 * minimal — we just enforce "not empty" and a sane ceiling.
 */
export const apiKeyNameSchema = z
  .string()
  .min(1, "Name is required")
  .max(200, "Name must be 200 characters or fewer")

/**
 * Service label: descriptive text shown in the keys table. Reasonable
 * human-friendly bounds; backend does not enforce a character set.
 */
export const serviceLabelSchema = z
  .string()
  .min(1, "Label is required")
  .max(200, "Label must be 200 characters or fewer")

/** Allowed agent-isolation platform identifiers. Empty = none set. */
export const PLATFORMS = [
  "claude-code",
  "cursor",
  "codex",
  "openclaw",
  "generic",
] as const

export const platformSchema = z.union([z.literal(""), z.enum(PLATFORMS)])

const aiKeyPrefillShape = {
  slug: z.string().optional(),
  label: z.string().optional(),
  via_node: z.string().optional(),
  org_id: z.string().uuid().optional(),
  endpoint_url: z.string().optional(),
  custom: z.boolean().optional(),
  custom_slug: z.string().optional(),
  auth_method: z.string().optional(),
  auth_key_name: z.string().optional(),
  // Manage-scopes mode (NyxID#917 follow-up): existing connection key id to
  // re-authorize with a new scope set. Set by `nyxid service scopes <slug>`.
  reconnect_key_id: z.string().optional(),
  // Declarative desired scope set (`--set`): pre-seeds the manage-scopes
  // picker with exactly these scopes.
  scope_override: z.array(z.string()).optional(),
} as const

export const aiKeyPrefillSchema = z.object(aiKeyPrefillShape)

export type ParsedAiKeyPrefill = z.infer<typeof aiKeyPrefillSchema>

export function parseAiKeyPrefill(input: unknown): ParsedAiKeyPrefill {
  if (!input || typeof input !== "object" || Array.isArray(input)) return {}

  const source = input as Record<string, unknown>
  const output: Partial<Record<keyof typeof aiKeyPrefillShape, unknown>> = {}
  for (const key of Object.keys(aiKeyPrefillShape) as Array<
    keyof typeof aiKeyPrefillShape
  >) {
    if (!Object.prototype.hasOwnProperty.call(source, key)) continue
    const parsed = aiKeyPrefillShape[key].safeParse(source[key])
    if (parsed.success && parsed.data !== undefined) {
      output[key] = parsed.data
    }
  }
  return output as ParsedAiKeyPrefill
}

/**
 * Helper used by live-validation inputs: extracts the first error message
 * from a Zod safeParse result, or returns `null` if the value is valid.
 */
export function firstError(
  schema: z.ZodType<string>,
  value: string,
): string | null {
  const result = schema.safeParse(value)
  if (result.success) return null
  return result.error.issues[0]?.message ?? "Invalid value"
}
