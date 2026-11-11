import { z } from "zod";
const rule = z.enum(["required", "optional", "forbidden"]);
export const contractSchema = z.object({
  version: z.string(),
  family: z.string(),
  fixture_sha256: z.string().regex(/^[a-f0-9]{64}$/),
  tokens: z.boolean(),
  cache_read: z.boolean(),
  split_reasoning: z.boolean(),
  bill_tool_input: z.boolean(),
  images: z.boolean(),
  requests: z.boolean(),
  cache_write_ttls: z.array(z.string()),
  input_modalities: z.array(z.string()),
  output_modalities: z.array(z.string()),
});
export const certifiedContractSchema = z.object({
  version: z.string(),
  family: z.string(),
  transports: z.array(z.string()),
  fixture_sha256: z.string().regex(/^[a-f0-9]{64}$/),
  flags: z.object({
    tokens: rule,
    cache_read: rule,
    split_reasoning: rule,
    bill_tool_input: rule,
    images: rule,
    requests: rule,
  }),
  cache_write_ttls: z.array(z.string()),
  input_modalities: z.array(z.string()),
  output_modalities: z.array(z.string()),
  default_contract: contractSchema,
});
export type CertifiedContract = z.infer<typeof certifiedContractSchema>;
export type UsageContract = z.infer<typeof contractSchema>;
export function attestation(publicModel: string, reference: string) {
  return `contract-attested-v1:${publicModel} ${reference.trim()}`;
}
export function validSource(source: string) {
  return (
    source.trim().length > 0 &&
    new TextEncoder().encode(source).length <= 1000 &&
    !Array.from(source).some(
      (c) =>
        c.charCodeAt(0) < 32 ||
        (c.charCodeAt(0) >= 127 && c.charCodeAt(0) <= 159),
    )
  );
}
