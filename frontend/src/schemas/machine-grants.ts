import { z } from "zod";

export const machineGrantCapabilitiesSchema = z.object({
  shell: z.boolean(),
  files: z.boolean(),
  browser: z.boolean(),
  computer: z.boolean(),
  developer_browser: z.boolean(),
});
const normalizedRoot = z.string().min(1).max(4096).refine(
  (root) => root === "/" || (root.startsWith("/") && !root.endsWith("/") && !root.includes("\\") && !root.includes("\0") && !root.slice(1).split("/").some((part) => !part || part === "." || part === "..")),
  "Use normalized absolute paths without trailing slashes or . / .. segments",
);
export const machineGrantAssignmentSchema = z.object({
  capabilities: machineGrantCapabilitiesSchema.refine(
    (c) => (c.shell || c.files) && !c.browser && !c.computer && !c.developer_browser,
    "Choose commands or files; external access supports only these capabilities",
  ),
  file_roots: z.array(normalizedRoot).min(1).max(16),
  service_ids: z.array(z.uuid()).max(100),
}).refine((a) => a.capabilities.shell || a.service_ids.length === 0, {
  message: "Gateway services require commands permission",
  path: ["service_ids"],
});
export const machineGrantSelectionSchema = z.object({
  assignments: z.record(z.uuid(), machineGrantAssignmentSchema).refine(
    (rows) => Object.keys(rows).length > 0 && Object.keys(rows).length <= 100,
    "Choose between one and 100 Machines",
  ),
  expires_at: z.iso.datetime().refine((value) => new Date(value).getTime() > Date.now(), "Grant expiry must be in the future").nullable().optional(),
});
export type MachineGrantSelection = z.infer<typeof machineGrantSelectionSchema>;
export const machineGrantSchema = z.object({
  id: z.string(),
  source: z.discriminatedUnion("kind", [
    z.object({ kind: z.literal("oauth"), oauth_client_id: z.string(), consent_id: z.string(), consent_revision: z.string() }),
    z.object({ kind: z.literal("api_key"), api_key_id: z.string(), api_key_state_version_at_approval: z.number().int() }),
  ]),
  revision: z.number().int(),
  policy_revision: z.number().int(),
  status: z.enum(["active", "revoked"]),
  expires_at: z.iso.datetime({ offset: true }),
  created_at: z.string(),
  updated_at: z.string(),
  revoked_at: z.string().nullable(),
  revoked_reason: z.string().nullable(),
  assignments: z.record(z.string(), z.object({
    capabilities: machineGrantCapabilitiesSchema,
    mode: z.string(),
    revision: z.number().int(),
    saved_login_ids: z.array(z.string()),
    file_roots: z.array(z.string()),
    service_ids: z.array(z.string()),
  })),
});
export type MachineGrant = z.infer<typeof machineGrantSchema>;
export const machineGrantValidityReasonSchema = z.enum([
  "missing",
  "disabled",
  "expired",
  "revoked",
  "changed",
  "ineligible",
]);
export type MachineGrantValidityReason = z.infer<typeof machineGrantValidityReasonSchema>;
export const machineGrantReadSchema = z.object({
  grant: machineGrantSchema.nullable(),
  // The independent fields are additive. Their absence remains unknown on
  // older servers; the combined field is not an independent validity result.
  source_valid: z.boolean().optional(),
  source_validity_reason: machineGrantValidityReasonSchema.nullable().optional(),
  grant_valid: z.boolean().optional(),
  grant_validity_reason: machineGrantValidityReasonSchema.nullable().optional(),
  source_and_grant_valid: z.boolean(),
}).superRefine((value, context) => {
  const independent = [value.source_valid, value.source_validity_reason, value.grant_valid, value.grant_validity_reason];
  if (independent.every((field) => field === undefined)) return;
  const complete = independent.every((field) => field !== undefined);
  const consistentReasons = (value.source_valid === true ? value.source_validity_reason === null : value.source_validity_reason != null)
    && (value.grant_valid === true ? value.grant_validity_reason === null : value.grant_validity_reason != null);
  if (!complete || !consistentReasons || value.source_and_grant_valid !== (value.source_valid === true && value.grant_valid === true)) {
    context.addIssue({ code: "custom", message: "Machine validity checks are incomplete or inconsistent" });
  }
});
export type MachineGrantSource = { kind: "api_key" | "oauth"; id: string };

// Display only. The server verifies the signed request at prepare and decision.
export function readMachineConsentScope(token: string): { scope: string; expiresAt: number; clientId: string } | null {
  try {
    const encoded = token.split(".")[1] ?? "";
    const bytes = Uint8Array.from(atob(encoded.replace(/-/g, "+").replace(/_/g, "/")), (char) => char.charCodeAt(0));
    const result = z.object({ token_type: z.literal("oauth_consent_request"), scope: z.string(), exp: z.number(), client_id: z.string().min(1) }).safeParse(JSON.parse(new TextDecoder().decode(bytes)));
    return result.success ? { scope: result.data.scope, expiresAt: result.data.exp * 1000, clientId: result.data.client_id } : null;
  } catch { return null; }
}

/** Mounted owner aggregate. Rows include retained history, not a live source check. */
export const machineGrantListSchema = z.object({
  external_grants: z.array(machineGrantSchema),
  next_cursor: z.string().min(1).nullable().optional(),
});
