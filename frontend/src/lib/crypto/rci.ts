import { x25519 } from "@noble/curves/ed25519.js";
import { xchacha20poly1305 } from "@noble/ciphers/chacha.js";
import { randomBytes } from "@noble/ciphers/utils.js";
import { hkdf } from "@noble/hashes/hkdf.js";
import { sha256 } from "@noble/hashes/sha2.js";
import { clean } from "@noble/hashes/utils.js";
import { decodeBase64UrlNoPadExact, encodeBase64UrlNoPad } from "./base64url";

export const VERSION_V1 = "v1";
export const MAX_CIPHERTEXT_SIZE = 16 * 1024;

const KDF_PREFIX = new TextEncoder().encode("nyxid:rci:v1:kdf\0");
const AAD_PREFIX = new TextEncoder().encode("nyxid:rci:v1:aad\0");
const textEncoder = new TextEncoder();

export type RciVersion = typeof VERSION_V1;

export interface RciContextFields {
  readonly node_id: string;
  readonly pending_credential_id: string;
  readonly service_slug: string;
  readonly injection_method: string;
  readonly field_name: string;
  readonly target_url?: string | null;
  readonly version?: RciVersion;
}

export interface RciContext {
  readonly node_id: string;
  readonly pending_credential_id: string;
  readonly service_slug: string;
  readonly injection_method: string;
  readonly field_name: string;
  readonly target_url: string | null;
  readonly version: RciVersion;
  readonly kdfInfoBytes: () => Uint8Array;
  readonly aadBytes: () => Uint8Array;
}

export interface CiphertextEnvelope {
  readonly version: RciVersion;
  readonly admin_pubkey: string;
  readonly nonce: string;
  readonly ciphertext: string;
}

export interface EphemeralKeypair {
  readonly secretKey: Uint8Array;
  readonly publicKey: Uint8Array;
}

function concatBytes(parts: readonly Uint8Array[]): Uint8Array {
  const length = parts.reduce((sum, part) => sum + part.length, 0);
  const out = new Uint8Array(length);
  let offset = 0;
  for (const part of parts) {
    out.set(part, offset);
    offset += part.length;
  }
  return out;
}

function lengthPrefixed(field: string, value: string): Uint8Array {
  const bytes = textEncoder.encode(value);
  if (bytes.length > 0xffff) {
    throw new Error(`${field} must be 65535 bytes or fewer`);
  }
  const out = new Uint8Array(bytes.length + 2);
  out[0] = bytes.length >>> 8;
  out[1] = bytes.length & 0xff;
  out.set(bytes, 2);
  return out;
}

function normalizeVersion(version: RciContextFields["version"]): RciVersion {
  if (version === undefined || version === VERSION_V1) {
    return VERSION_V1;
  }
  throw new Error(`Unsupported RCI version: ${version}`);
}

function cloneBytes(bytes: Uint8Array): Uint8Array {
  return new Uint8Array(bytes);
}

export function generateEphemeralKeypair(): EphemeralKeypair {
  const keypair = x25519.keygen();
  return {
    secretKey: cloneBytes(keypair.secretKey),
    publicKey: cloneBytes(keypair.publicKey),
  };
}

export function buildRciContext(fields: RciContextFields): RciContext {
  const version = normalizeVersion(fields.version);
  const targetUrl = fields.target_url ?? null;

  return {
    node_id: fields.node_id,
    pending_credential_id: fields.pending_credential_id,
    service_slug: fields.service_slug,
    injection_method: fields.injection_method,
    field_name: fields.field_name,
    target_url: targetUrl,
    version,
    kdfInfoBytes: () =>
      concatBytes([
        KDF_PREFIX,
        lengthPrefixed("node_id", fields.node_id),
        lengthPrefixed("pending_credential_id", fields.pending_credential_id),
        lengthPrefixed("service_slug", fields.service_slug),
        lengthPrefixed("version", version),
      ]),
    aadBytes: () =>
      concatBytes([
        AAD_PREFIX,
        lengthPrefixed("node_id", fields.node_id),
        lengthPrefixed("pending_credential_id", fields.pending_credential_id),
        lengthPrefixed("service_slug", fields.service_slug),
        lengthPrefixed("injection_method", fields.injection_method),
        lengthPrefixed("field_name", fields.field_name),
        lengthPrefixed("target_url", targetUrl ?? ""),
        lengthPrefixed("version", version),
      ]),
  };
}

export function encrypt(
  plaintext: Uint8Array,
  nodePubkey: Uint8Array | string,
  rciContext: RciContext,
): CiphertextEnvelope {
  if (rciContext.version !== VERSION_V1) {
    throw new Error(`Unsupported RCI version: ${rciContext.version}`);
  }

  const recipientPubkey =
    typeof nodePubkey === "string"
      ? decodeBase64UrlNoPadExact(nodePubkey, "node_pubkey", 32)
      : nodePubkey;
  if (recipientPubkey.length !== 32) {
    throw new Error("node_pubkey must be 32 bytes");
  }

  const nonce = randomBytes(24);
  const adminKeypair = generateEphemeralKeypair();
  const sharedSecret = x25519.getSharedSecret(
    adminKeypair.secretKey,
    recipientPubkey,
  );
  const key = hkdf(
    sha256,
    sharedSecret,
    undefined,
    rciContext.kdfInfoBytes(),
    32,
  );

  try {
    const cipher = xchacha20poly1305(key, nonce, rciContext.aadBytes());
    const ciphertext = cipher.encrypt(plaintext);
    if (ciphertext.length > MAX_CIPHERTEXT_SIZE) {
      throw new Error(
        `ciphertext exceeds maximum size: ${String(ciphertext.length)} > ${String(MAX_CIPHERTEXT_SIZE)}`,
      );
    }

    return {
      version: VERSION_V1,
      admin_pubkey: encodeBase64UrlNoPad(adminKeypair.publicKey),
      nonce: encodeBase64UrlNoPad(nonce),
      ciphertext: encodeBase64UrlNoPad(ciphertext),
    };
  } finally {
    clean(adminKeypair.secretKey, sharedSecret, key);
  }
}
