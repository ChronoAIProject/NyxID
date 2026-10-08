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
  mode: z.enum(["shared_legacy", "separated"]).optional(),
  execution_note: z.string().optional(),
  separated_setup_note: z.string().nullish(),
  separated: z.object({ available: z.boolean(), landlock_abi: z.number().nullable(), reason: z.string().nullable() }).nullish(),
  revocation_pending: z.boolean().optional(),
  saved_login_ids: z.array(z.string()).nullable(),
});
export const machineSelectionSchema = z.object({
  expected_revision: z.number().int().positive(), capabilities: capabilitiesSchema,
  saved_login_ids: z.array(z.string()).max(64).nullable(),
}).refine(v => v.capabilities.browser || (!v.capabilities.computer && !v.capabilities.developer_browser), {
  message: "Browser is required for computer and developer browser access", path: ["capabilities", "browser"],
});
export const machineContextSelectionSchema = machineSelectionSchema.extend({
  mode: z.literal("separated"),
});
export const machineContextSchema = z.object({
  context_id: z.string().min(1),
  agent_id: z.string().min(1),
  agent_name: z.string().min(1),
  actor_label: z.string().min(1),
  group_id: z.string().nullable(),
  generation: z.number().int().positive(),
});
export type MachineAccess = z.infer<typeof machineAccessSchema>;
export type MachineSelection = z.infer<typeof machineSelectionSchema>;
export type MachineContextSelection = z.infer<typeof machineContextSelectionSchema>;
export type MachineContext = z.infer<typeof machineContextSchema>;

export const SEPARATED_SETUP_NOTE = "To enable separate workspaces and browsers, use a Linux machine with Landlock ABI 6+, separate OS users and a managed browser installation, then approve an owner action card. Browser permissions are not required for configured shared commands and file tools.";
