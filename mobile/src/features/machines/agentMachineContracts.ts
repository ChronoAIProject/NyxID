import { z } from "zod";
import { MachineReviewRequired } from "./machineContracts";

export const capabilityLabels = {
  shell: "Shell commands",
  files: "File tools",
  browser: "Secure browser",
  computer: "Full computer control",
  developer_browser: "Developer browser",
} as const;
export const machineCapabilitiesSchema = z
  .object({
    shell: z.boolean(),
    files: z.boolean(),
    browser: z.boolean(),
    computer: z.boolean(),
    developer_browser: z.boolean(),
  })
  .strict();
export type MachineCapabilities = z.infer<typeof machineCapabilitiesSchema>;
export const agentMachineSchema = z.object({
  node_id: z.string().min(1),
  name: z.string(),
  revision: z.number().int().positive(),
  protocol_v2: z.boolean(),
  can_edit: z.boolean().default(false),
  capabilities: machineCapabilitiesSchema,
  ceiling: machineCapabilitiesSchema,
  legacy: z.boolean(),
  mode: z.enum(["shared_legacy", "separated", "selection_required"]).optional(),
  selection_required: z.boolean().optional(),
  selection_reason: z.string().nullish(),
  separation_choice: z
    .enum(["automatic", "require_separated", "shared_legacy"])
    .nullish(),
  execution_note: z.string().optional(),
  separated_setup_note: z.string().nullish(),
  separated: z
    .object({
      available: z.boolean(),
      landlock_abi: z.number().nullable(),
      reason: z.string().nullable(),
    })
    .nullish(),
  revocation_pending: z.boolean().optional(),
  delivery_pending: z.boolean().optional(),
  quarantine_pending: z.boolean().optional(),
  execution_revocation_pending: z.boolean().optional(),
  revocation_initialization_pending: z.boolean().optional(),
  revocation_expires_at: z.string().nullish(),
  revocation_diagnostic: z
    .enum(["revocation_deadline_invalid", "authority_timing_upgrade_required"])
    .nullish(),
  saved_login_ids: z.string().array().nullable(),
});
export type AgentMachine = z.infer<typeof agentMachineSchema>;
export const machineSelectionSchema = z
  .object({
    expected_revision: z.number().int().positive(),
    capabilities: machineCapabilitiesSchema,
    saved_login_ids: z.string().array().max(64).nullable(),
  })
  .strict()
  .refine(
    (value) =>
      value.capabilities.browser ||
      (!value.capabilities.computer && !value.capabilities.developer_browser),
    {
      message:
        "Secure browser is required for computer and developer browser access.",
      path: ["capabilities", "browser"],
    },
  );
export const contextModeSchema = z.enum(["separated", "shared_legacy"]);
export type ContextMode = z.infer<typeof contextModeSchema>;
export const contextRequestSchema = z.object({
  status: z.literal("pending"),
  acknowledgement_id: z.string().min(1),
  conversation_id: z.string().min(1),
  message: z.string(),
});

export function selectionRequired(machine: AgentMachine) {
  return (
    machine.selection_required === true || machine.mode === "selection_required"
  );
}
export function separationAvailable(machine: AgentMachine) {
  return machine.separated?.available === true && !machine.separated.reason;
}
export function setMachineCapability(
  caps: MachineCapabilities,
  key: keyof MachineCapabilities,
  value: boolean,
): MachineCapabilities {
  return {
    ...caps,
    [key]: value,
    ...(key === "browser" && !value
      ? { computer: false, developer_browser: false }
      : {}),
  };
}
export function capabilitySelection(
  machine: AgentMachine,
  capabilities: MachineCapabilities,
) {
  const selection = machineSelectionSchema.parse({
    expected_revision: machine.revision,
    capabilities,
    saved_login_ids: machine.saved_login_ids,
  });
  if (!machine.can_edit || !machine.protocol_v2 || selectionRequired(machine))
    throw new MachineReviewRequired(
      "This machine needs owner selection, current protocol support and editing authority before capabilities can change.",
    );
  for (const key of Object.keys(
    capabilityLabels,
  ) as (keyof MachineCapabilities)[]) {
    if (capabilities[key] && !machine.ceiling[key])
      throw new MachineReviewRequired(
        `${capabilityLabels[key]} is off on the machine.`,
      );
  }
  return selection;
}
export function machineAccessState(machine: AgentMachine) {
  const states: string[] = [];
  if (selectionRequired(machine))
    states.push(
      machine.selection_reason ??
        "Execution is blocked until the owner approves a supported separation choice or explicitly chooses shared legacy access.",
    );
  if (machine.mode === "shared_legacy" || machine.legacy)
    states.push(
      "Shared legacy access: workspace and browser sessions may be shared with other agents.",
    );
  if (machine.mode === "separated" && !separationAvailable(machine))
    states.push(
      "Separated execution is unavailable and refused. There is no automatic shared fallback.",
    );
  if (machine.quarantine_pending)
    states.push(
      "Browser cleanup is pending. New browser work remains blocked until cleanup finishes.",
    );
  if (machine.revocation_initialization_pending)
    states.push(
      "Checking when earlier commands will stop. Refresh status shortly.",
    );
  else if (machine.execution_revocation_pending ?? machine.revocation_pending)
    states.push(
      `Earlier commands are stopping.${machine.revocation_expires_at ? ` Expected deadline: ${machine.revocation_expires_at}.` : " Reconnect and refresh to confirm they have stopped."}`,
    );
  else if (machine.delivery_pending)
    states.push(
      "Earlier commands have stopped. Updated permissions still need machine acknowledgement.",
    );
  if (machine.revocation_diagnostic)
    states.push(
      machine.revocation_diagnostic === "authority_timing_upgrade_required"
        ? "Update and reconnect this machine to confirm permission changes."
        : "Recovery needs administrator attention. Earlier commands are still marked as stopping.",
    );
  return states;
}
export const SHARED_LEGACY_WARNING =
  "Shared legacy access gives commands the full access of their OS user. A misbehaving or prompt-injected agent may read accessible node tokens, signing secrets and stored credentials. Workspace roots constrain file tools, not the shell. Separated contexts still share their host kernel; use a separate machine container or VM per agent for stronger isolation.";
