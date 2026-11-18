import { z } from "zod";
import { accessModeSchema } from "../agents/agentContracts";

export type ComposerAccessMode = z.infer<typeof accessModeSchema>;
export const composerSettingsSchema = z.object({
  access_mode: accessModeSchema.default("nyxmode"),
});
export const COMPOSER_ACCESS_LABEL = {
  autopilot: "Full access",
  nyxmode: "Auto",
  ask_every_time: "Ask every time",
} as const satisfies Record<ComposerAccessMode, string>;

export const COMPOSER_ACCESS_DESCRIPTION = {
  autopilot:
    "Never asks. Every action, including automation, machine, saved-login and service-approval confirmations, runs on its own and the chat records what was allowed automatically.",
  nyxmode: "Acts on its own; asks before sensitive or destructive actions.",
  ask_every_time:
    "Asks before every action that changes something. Reading never asks.",
} as const satisfies Record<ComposerAccessMode, string>;
