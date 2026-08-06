const DEFAULT_TIMESTAMP_TOLERANCE_SECONDS = 300;
const SIGNATURE_PATTERN = /^sha256=([0-9a-f]{64})$/i;

export interface VerifyWebhookSignatureInput {
  readonly secret: string;
  readonly timestamp: string;
  readonly rawBody: string | Uint8Array;
  readonly signatureHeader: string;
  /** Maximum accepted timestamp skew in seconds. Defaults to 300 seconds. */
  readonly toleranceSeconds?: number;
  /** Current Unix time override for deterministic tests. */
  readonly nowSeconds?: number;
}

/**
 * Verifies a NyxID connection-webhook signature and its replay window.
 * Pass the unmodified request body and the exact values from
 * `X-NyxID-Timestamp` and `X-NyxID-Signature`. Mismatches return `false`.
 */
export async function verifyConnectionWebhookSignature(
  input: VerifyWebhookSignatureInput,
): Promise<boolean> {
  return verifyTimestampedSignature(input);
}

/**
 * Verifies an outbound trigger-webhook signature. Trigger and connection
 * webhooks share the same timestamp-bound HMAC-SHA256 wire contract.
 */
export async function verifyTriggerWebhookSignature(
  input: VerifyWebhookSignatureInput,
): Promise<boolean> {
  return verifyTimestampedSignature(input);
}

async function verifyTimestampedSignature(
  input: VerifyWebhookSignatureInput,
): Promise<boolean> {
  try {
    const match = SIGNATURE_PATTERN.exec(input.signatureHeader);
    if (!match || !input.secret) return false;

    const timestampSeconds = Number(input.timestamp);
    const nowSeconds = input.nowSeconds ?? Math.floor(Date.now() / 1_000);
    const toleranceSeconds =
      input.toleranceSeconds ?? DEFAULT_TIMESTAMP_TOLERANCE_SECONDS;
    if (
      !Number.isSafeInteger(timestampSeconds) ||
      timestampSeconds < 0 ||
      !Number.isFinite(nowSeconds) ||
      !Number.isFinite(toleranceSeconds) ||
      toleranceSeconds < 0 ||
      Math.abs(nowSeconds - timestampSeconds) > toleranceSeconds
    ) {
      return false;
    }

    const subtle = globalThis.crypto?.subtle;
    if (!subtle) return false;

    const encoder = new TextEncoder();
    const key = await subtle.importKey(
      "raw",
      encoder.encode(input.secret),
      { name: "HMAC", hash: "SHA-256" },
      false,
      ["verify"],
    );
    const rawBody =
      typeof input.rawBody === "string"
        ? encoder.encode(input.rawBody)
        : input.rawBody;
    const signedContent = joinSignedContent(
      encoder.encode(`${input.timestamp}.`),
      rawBody,
    );
    return await subtle.verify("HMAC", key, decodeHex(match[1]), signedContent);
  } catch {
    return false;
  }
}

function joinSignedContent(
  prefix: Uint8Array,
  body: Uint8Array,
): Uint8Array<ArrayBuffer> {
  const content = new Uint8Array(prefix.byteLength + body.byteLength);
  content.set(prefix);
  content.set(body, prefix.byteLength);
  return content;
}

function decodeHex(value: string): Uint8Array<ArrayBuffer> {
  const bytes = new Uint8Array(value.length / 2);
  for (let index = 0; index < bytes.length; index += 1) {
    bytes[index] = Number.parseInt(value.slice(index * 2, index * 2 + 2), 16);
  }
  return bytes;
}
