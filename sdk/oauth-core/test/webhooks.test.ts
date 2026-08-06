import { describe, expect, it } from "vitest";
import {
  verifyConnectionWebhookSignature,
  verifyTriggerWebhookSignature,
} from "../src/index.js";

const BACKEND_FIXTURE = {
  secret: "fixture-secret",
  timestamp: "1700000000",
  rawBody: '{"event_type":"connect_link.completed"}',
  signatureHeader:
    "sha256=b426d8e45504ab2702700a6ac32e73f9355bcbcfae7ffd6d452b65a50755b617",
} as const;

describe("webhook signature verification", () => {
  it("matches the backend compute_timestamped_signature fixture byte-for-byte", async () => {
    await expect(
      verifyConnectionWebhookSignature({
        ...BACKEND_FIXTURE,
        nowSeconds: 1_700_000_100,
      }),
    ).resolves.toBe(true);
    await expect(
      verifyTriggerWebhookSignature({
        ...BACKEND_FIXTURE,
        nowSeconds: 1_700_000_100,
      }),
    ).resolves.toBe(true);
  });

  it("accepts timestamps inside the default replay window and rejects stale ones", async () => {
    await expect(
      verifyConnectionWebhookSignature({
        ...BACKEND_FIXTURE,
        nowSeconds: 1_700_000_300,
      }),
    ).resolves.toBe(true);
    await expect(
      verifyConnectionWebhookSignature({
        ...BACKEND_FIXTURE,
        nowSeconds: 1_700_000_301,
      }),
    ).resolves.toBe(false);
  });

  it("supports a custom tolerance and returns false without throwing on mismatch", async () => {
    await expect(
      verifyConnectionWebhookSignature({
        ...BACKEND_FIXTURE,
        signatureHeader: `${BACKEND_FIXTURE.signatureHeader.slice(0, -1)}0`,
        toleranceSeconds: 1_000,
        nowSeconds: 1_700_000_500,
      }),
    ).resolves.toBe(false);
    await expect(
      verifyConnectionWebhookSignature({
        ...BACKEND_FIXTURE,
        timestamp: "invalid",
      }),
    ).resolves.toBe(false);
  });

  it("selects rotation secrets by the emitted key id", async () => {
    const fixture = {
      timestamp: BACKEND_FIXTURE.timestamp,
      rawBody: BACKEND_FIXTURE.rawBody,
      signatureHeader: BACKEND_FIXTURE.signatureHeader,
    };
    await expect(
      verifyConnectionWebhookSignature({
        ...fixture,
        keyId: "key_current",
        secretsByKeyId: {
          key_previous: "previous-secret",
          key_current: BACKEND_FIXTURE.secret,
        },
        nowSeconds: 1_700_000_100,
      }),
    ).resolves.toBe(true);
    await expect(
      verifyTriggerWebhookSignature({
        ...fixture,
        keyId: "key_unknown",
        secretsByKeyId: { key_current: BACKEND_FIXTURE.secret },
        nowSeconds: 1_700_000_100,
      }),
    ).resolves.toBe(false);
  });
});
