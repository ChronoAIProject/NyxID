export const RELEASE_INTEGRITY_SCHEMA_VERSION = "nyxid.release-integrity.v1";
export const CREDENTIAL_ACCEPT_FINGERPRINT_PREFIX = "nyxid:rci-accept:v1\0";
export const CREDENTIAL_ACCEPT_SCRIPT_ROLE = "credential_accept_script";
export const CREDENTIAL_ACCEPT_HTML_ROLE = "credential_accept_html";

const textEncoder = new TextEncoder();

export type ReleaseIntegrityArtifactRole =
  | typeof CREDENTIAL_ACCEPT_HTML_ROLE
  | typeof CREDENTIAL_ACCEPT_SCRIPT_ROLE;

export interface ReleaseIntegrityArtifact {
  readonly role: ReleaseIntegrityArtifactRole;
  readonly path: string;
  readonly content_type: string;
  readonly size_bytes: number;
  readonly sha384_sri: string;
  readonly sha384_hex: string;
}

export interface ReleaseIntegrityManifest {
  readonly schema_version: typeof RELEASE_INTEGRITY_SCHEMA_VERSION;
  readonly app_version: string;
  readonly git_commit: string;
  readonly generated_at: string;
  readonly credential_accept: {
    readonly fingerprint_sha384_hex: string;
  };
  readonly artifacts: readonly ReleaseIntegrityArtifact[];
}

export interface CredentialAcceptScriptBytes {
  readonly path: string;
  readonly bytes: Uint8Array;
}

export function bytesToHex(bytes: Uint8Array): string {
  return Array.from(bytes, (byte) => byte.toString(16).padStart(2, "0")).join(
    "",
  );
}

export function hexToBytes(hex: string): Uint8Array {
  if (!/^[0-9a-f]*$/.test(hex) || hex.length % 2 !== 0) {
    throw new Error("hex must be lowercase and even-length");
  }
  const out = new Uint8Array(hex.length / 2);
  for (let i = 0; i < out.length; i += 1) {
    out[i] = Number.parseInt(hex.slice(i * 2, i * 2 + 2), 16);
  }
  return out;
}

function compareUtf8PathBytes(a: string, b: string): number {
  const aBytes = textEncoder.encode(a);
  const bBytes = textEncoder.encode(b);
  const len = Math.min(aBytes.length, bBytes.length);
  for (let index = 0; index < len; index += 1) {
    const diff = (aBytes[index] ?? 0) - (bBytes[index] ?? 0);
    if (diff !== 0) return diff;
  }
  return aBytes.length - bBytes.length;
}

function u32be(value: number): Uint8Array {
  if (!Number.isSafeInteger(value) || value < 0 || value > 0xffffffff) {
    throw new Error("u32 length out of range");
  }
  const out = new Uint8Array(4);
  out[0] = (value >>> 24) & 0xff;
  out[1] = (value >>> 16) & 0xff;
  out[2] = (value >>> 8) & 0xff;
  out[3] = value & 0xff;
  return out;
}

function u64be(value: number): Uint8Array {
  if (!Number.isSafeInteger(value) || value < 0) {
    throw new Error("u64 length out of range");
  }
  const out = new Uint8Array(8);
  let remaining = BigInt(value);
  for (let index = 7; index >= 0; index -= 1) {
    out[index] = Number(remaining & 0xffn);
    remaining >>= 8n;
  }
  return out;
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

export function canonicalizeCredentialAcceptScripts(
  scripts: readonly CredentialAcceptScriptBytes[],
): Uint8Array {
  const parts: Uint8Array[] = [textEncoder.encode(CREDENTIAL_ACCEPT_FINGERPRINT_PREFIX)];
  const sorted = [...scripts].sort((a, b) => compareUtf8PathBytes(a.path, b.path));

  for (const script of sorted) {
    const pathBytes = textEncoder.encode(script.path);
    parts.push(u32be(pathBytes.length), pathBytes, u64be(script.bytes.length), script.bytes);
  }

  return concatBytes(parts);
}

export async function sha384Hex(bytes: Uint8Array): Promise<string> {
  const stableBytes = new Uint8Array(bytes.length);
  stableBytes.set(bytes);
  const digest = await crypto.subtle.digest("SHA-384", stableBytes.buffer);
  return bytesToHex(new Uint8Array(digest));
}

export async function credentialAcceptFingerprintSha384Hex(
  scripts: readonly CredentialAcceptScriptBytes[],
): Promise<string> {
  return sha384Hex(canonicalizeCredentialAcceptScripts(scripts));
}

export function pathFromSameOriginScriptUrl(src: string, baseUrl: string): string {
  const url = new URL(src, baseUrl);
  const base = new URL(baseUrl);
  if (url.origin !== base.origin) {
    throw new Error("credential accept script must be same-origin");
  }
  return url.pathname;
}

export function isValidSha384Hex(value: string): boolean {
  return /^[0-9a-f]{96}$/.test(value);
}
