import { expect, it } from "vitest";
import {
  gatewayChannelIdSchema,
  nativeSetupRecoverySchema,
} from "./native-platform-entry";
const actualChannelId =
  "cmaeg1.ch.1.FGk0KoqKvJ7PFnznQDfWsk3v3LtVLlkp-Ue3MZ9zF646sV8XE8PQBQ8OHk4OUxJFUCZO8XLK5EWgn7JSKsS3qkBu0N_e16E4bDiL5JGLZPQEXShqkOqBh8RIGvWghhLeLqZdQUMdd_vsl47KdXpz89rqTDaQz5YQ0SaOUrKMrCp_8jhGjWUntBAItGMT2nADcwoIxl1EN1fGmM9LNPYWRjw70yhQndZo2aFi6qunxKYoYmZWFPR1hpAqe55b";
it("accepts the actual 264-byte CMA sealed pointer in an exact retained review", () => {
  expect(actualChannelId.length).toBe(264);
  expect(gatewayChannelIdSchema.parse(actualChannelId)).toBe(actualChannelId);
  expect(
    nativeSetupRecoverySchema.safeParse({
      kind: "gateway",
      record_id: "11111111-1111-4111-8111-111111111111",
      attempt_id: "22222222-2222-4222-8222-222222222222",
      revision: 1,
      request_hash: `sha256:${"a".repeat(64)}`,
      credential_fingerprint: "b".repeat(64),
      remote_channel_id: actualChannelId,
      remote_version: 1,
      remote_ready: true,
    }).success,
  ).toBe(true);
});
it("matches the backend's canonical u8 and unpadded ciphertext grammar", () => {
  const payload = btoa("x".repeat(28)).replace(/=+$/, "");
  for (const kid of ["0", "1", "255"])
    expect(
      gatewayChannelIdSchema.safeParse(`cmaeg1.ch.${kid}.${payload}`).success,
    ).toBe(true);
  for (const id of [
    `cmaeg1.ch.256.${payload}`,
    `cmaeg1.ch.01.${payload}`,
    `cmaeg1.ch.+1.${payload}`,
    "cmaeg1.ch.1.a",
    `cmaeg1.ch.1.${payload}=`,
    `cmaeg1.ch.1.${btoa("x".repeat(27))}`,
    `cmaeg1.ch.1.${"A".repeat(502)}`,
    `cmaeg1.ch.1.${payload}.extra`,
    `cmaeg1.ch.1.${payload.slice(0, -1)}B`,
  ])
    expect(gatewayChannelIdSchema.safeParse(id).success, id).toBe(false);
});
