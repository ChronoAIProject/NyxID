import { readFileSync } from "node:fs";
import { describe, expect, it } from "vitest";
import { x25519 } from "@noble/curves/ed25519.js";
import { xchacha20poly1305 } from "@noble/ciphers/chacha.js";
import { hkdf } from "@noble/hashes/hkdf.js";
import { sha256 } from "@noble/hashes/sha2.js";
import {
  VERSION_V1,
  buildRciContext,
  decodeBase64UrlNoPad,
  decodeBase64UrlNoPadExact,
  encodeBase64UrlNoPad,
} from "../src/lib/crypto";

interface Fixture {
  readonly node_private_key: string;
  readonly node_public_key: string;
  readonly admin_private_key: string;
  readonly nonce: string;
  readonly plaintext: string;
  readonly context: {
    readonly node_id: string;
    readonly pending_credential_id: string;
    readonly service_slug: string;
    readonly injection_method: string;
    readonly field_name: string;
    readonly target_url: string | null;
    readonly version: "v1";
  };
  readonly envelope: {
    readonly version: "v1";
    readonly admin_pubkey: string;
    readonly nonce: string;
    readonly ciphertext: string;
  };
}

const fixture = JSON.parse(
  readFileSync("../tests/fixtures/rci/v1_envelope.json", "utf8"),
) as Fixture;

function deterministicEncrypt() {
  const context = buildRciContext(fixture.context);
  const plaintext = new TextEncoder().encode(fixture.plaintext);
  const nodePubkey = decodeBase64UrlNoPadExact(
    fixture.node_public_key,
    "node_public_key",
    32,
  );
  const adminPrivateKey = decodeBase64UrlNoPadExact(
    fixture.admin_private_key,
    "admin_private_key",
    32,
  );
  const nonce = decodeBase64UrlNoPadExact(fixture.nonce, "nonce", 24);
  const sharedSecret = x25519.getSharedSecret(adminPrivateKey, nodePubkey);
  const key = hkdf(sha256, sharedSecret, undefined, context.kdfInfoBytes(), 32);
  const ciphertext = xchacha20poly1305(key, nonce, context.aadBytes()).encrypt(
    plaintext,
  );

  return {
    version: VERSION_V1,
    admin_pubkey: encodeBase64UrlNoPad(x25519.getPublicKey(adminPrivateKey)),
    nonce: encodeBase64UrlNoPad(nonce),
    ciphertext: encodeBase64UrlNoPad(ciphertext),
  };
}

function decryptFixture(): Uint8Array {
  const context = buildRciContext(fixture.context);
  const nodePrivateKey = decodeBase64UrlNoPadExact(
    fixture.node_private_key,
    "node_private_key",
    32,
  );
  const adminPubkey = decodeBase64UrlNoPadExact(
    fixture.envelope.admin_pubkey,
    "admin_pubkey",
    32,
  );
  const nonce = decodeBase64UrlNoPadExact(fixture.envelope.nonce, "nonce", 24);
  const ciphertext = decodeBase64UrlNoPad(
    fixture.envelope.ciphertext,
    "ciphertext",
  );
  const sharedSecret = x25519.getSharedSecret(nodePrivateKey, adminPubkey);
  const key = hkdf(sha256, sharedSecret, undefined, context.kdfInfoBytes(), 32);
  return xchacha20poly1305(key, nonce, context.aadBytes()).decrypt(ciphertext);
}

describe("RCI v1 Rust/JS interop fixture", () => {
  it("decrypts the Rust fixture", () => {
    expect(new TextDecoder().decode(decryptFixture())).toBe(fixture.plaintext);
  });

  it("matches deterministic fixture encryption", () => {
    expect(deterministicEncrypt()).toEqual(fixture.envelope);
  });

  it("keeps KDF and AAD vectors stable", () => {
    const context = buildRciContext(fixture.context);
    expect(encodeBase64UrlNoPad(context.kdfInfoBytes())).toBe(
      "bnl4aWQ6cmNpOnYxOmtkZgAACG5vZGUtMTIzAAtwZW5kaW5nLTQ1NgAGb3BlbmFpAAJ2MQ",
    );
    expect(encodeBase64UrlNoPad(context.aadBytes())).toBe(
      "bnl4aWQ6cmNpOnYxOmFhZAAACG5vZGUtMTIzAAtwZW5kaW5nLTQ1NgAGb3BlbmFpAAZoZWFkZXIADUF1dGhvcml6YXRpb24AGWh0dHBzOi8vYXBpLm9wZW5haS5jb20vdjEAAnYx",
    );
  });
});
