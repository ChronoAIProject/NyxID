import { translateApp } from "../../i18n/appTranslation";
import { z } from "zod";
import { accessModeSchema } from "../agents/agentContracts";

export type ComposerAccessMode = z.infer<typeof accessModeSchema>;
export const composerSettingsSchema = z.object({
  access_mode: accessModeSchema.default("nyxmode"),
});
export const COMPOSER_ACCESS_LABEL = {
  get autopilot() {
    return translateApp("Full access");
  },
  get nyxmode() {
    return translateApp("Auto");
  },
  get ask_every_time() {
    return translateApp("Ask every time");
  },
} as const satisfies Record<ComposerAccessMode, string>;

export const COMPOSER_ACCESS_DESCRIPTION = {
  get autopilot() {
    return translateApp(
      "Never asks. Every action, including automation, machine, saved-login and service-approval confirmations, runs on its own and the chat records what was allowed automatically.",
    );
  },
  get nyxmode() {
    return translateApp(
      "Acts on its own; asks before sensitive or destructive actions.",
    );
  },
  get ask_every_time() {
    return translateApp(
      "Asks before every action that changes something. Reading never asks.",
    );
  },
} as const satisfies Record<ComposerAccessMode, string>;
