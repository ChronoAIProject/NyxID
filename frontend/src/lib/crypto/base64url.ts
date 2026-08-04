const BASE64URL_RE = /^[A-Za-z0-9_-]*$/;

function bytesToBinary(bytes: Uint8Array): string {
  let out = "";
  const chunkSize = 0x8000;
  for (let offset = 0; offset < bytes.length; offset += chunkSize) {
    out += String.fromCharCode(...bytes.subarray(offset, offset + chunkSize));
  }
  return out;
}

function binaryToBytes(binary: string): Uint8Array {
  const out = new Uint8Array(binary.length);
  for (let index = 0; index < binary.length; index += 1) {
    out[index] = binary.charCodeAt(index);
  }
  return out;
}

export function encodeBase64UrlNoPad(bytes: Uint8Array): string {
  return btoa(bytesToBinary(bytes))
    .replaceAll("+", "-")
    .replaceAll("/", "_")
    .replace(/=+$/u, "");
}

export function decodeBase64UrlNoPad(value: string, field: string): Uint8Array {
  if (value.includes("=")) {
    throw new Error(`${field} must be base64url without padding`);
  }
  if (!BASE64URL_RE.test(value) || value.length % 4 === 1) {
    throw new Error(`${field} must be valid base64url`);
  }

  const padded = value
    .replaceAll("-", "+")
    .replaceAll("_", "/")
    .padEnd(Math.ceil(value.length / 4) * 4, "=");

  try {
    return binaryToBytes(atob(padded));
  } catch {
    throw new Error(`${field} must be valid base64url`);
  }
}

export function decodeBase64UrlNoPadExact(
  value: string,
  field: string,
  expectedLength: number,
): Uint8Array {
  const decoded = decodeBase64UrlNoPad(value, field);
  if (decoded.length !== expectedLength) {
    throw new Error(`${field} must decode to ${String(expectedLength)} bytes`);
  }
  return decoded;
}
