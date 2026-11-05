import { z } from "zod";
import { unitPriceSchema } from "@/schemas/billing-metrics";

export const CMA_RESOURCE_LABELS = {
  sandbox_cpu_millis_seconds: "CPU (credits per millicore-second)",
  sandbox_memory_mib_seconds: "RAM (credits per MiB-second)",
  sandbox_disk_gib_seconds: "Retained disk (credits per GiB-second)",
} as const;
export const cmaRatesSchema = z
  .object({
    sandbox_cpu_millis_seconds: unitPriceSchema,
    sandbox_memory_mib_seconds: unitPriceSchema,
    sandbox_disk_gib_seconds: unitPriceSchema,
  })
  .refine(
    (rates) => Object.values(rates).every((rate) => /[1-9]/.test(rate)),
    "Every prepaid resource rate must be positive",
  );
export const cmaRateCardSchema = z.object({
  id: z.string(),
  version: z.number().int().positive(),
  effective_from: z.string(),
  rates: cmaRatesSchema,
  sync_status: z.enum(["pending", "synced", "failed"]),
  created_at: z.string(),
  created_by: z.string().nullable(),
});
export const cmaRateFormSchema = z.object({
  effective_from: z
    .string()
    .refine(
      (value) => Number.isFinite(Date.parse(value)),
      "Choose an effective date and time",
    ),
  rates: cmaRatesSchema,
});
export type CmaRateForm = z.infer<typeof cmaRateFormSchema>;
export type CmaRateCard = z.infer<typeof cmaRateCardSchema>;
export type PublishCmaRateCard = {
  expected_version: number;
  effective_from: string;
  rates: CmaRateForm["rates"];
};
