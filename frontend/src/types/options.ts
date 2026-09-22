import type { z } from "zod";
import type { optionItemSchema, optionsResponseSchema } from "@/schemas/options";

export type OptionItem = z.infer<typeof optionItemSchema>;
export type OptionsResponse = z.infer<typeof optionsResponseSchema>;
export type OptionSet = OptionsResponse["option_set"];
export interface OptionsContext {
  readonly principal_type: "service_account";
  readonly owner_id: string;
  readonly service_account_id?: string;
}
