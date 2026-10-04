import { z } from "zod";
export const capabilitiesSchema = z.object({
  shell: z.boolean(), files: z.boolean(),
  browser: z.boolean(), computer: z.boolean(),
  developer_browser: z.boolean(),
});
export const machineAccessSchema = z.object({
  node_id: z.string(), name: z.string(), revision: z.number(), protocol_v2: z.boolean(),
  capabilities: capabilitiesSchema, ceiling: capabilitiesSchema, legacy: z.boolean(),
  can_edit: z.boolean().default(false),
  revocation_pending: z.boolean().optional(),
  saved_login_ids: z.array(z.string()).nullable(),
});
export const machineSelectionSchema = z.object({
  expected_revision: z.number().int().positive(), capabilities: capabilitiesSchema,
  saved_login_ids: z.array(z.string()).max(64).nullable(),
}).refine(v => v.capabilities.browser || (!v.capabilities.computer && !v.capabilities.developer_browser), {
  message: "Browser is required for computer and developer browser access", path: ["capabilities", "browser"],
});
export type MachineAccess = z.infer<typeof machineAccessSchema>;
export type MachineSelection = z.infer<typeof machineSelectionSchema>;
