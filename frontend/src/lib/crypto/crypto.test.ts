import { describe, expect, it } from "vitest";
import { x25519 } from "@noble/curves/ed25519.js";
import { xchacha20poly1305 } from "@noble/ciphers/chacha.js";
import { hkdf } from "@noble/hashes/hkdf.js";
import { sha256 } from "@noble/hashes/sha2.js";
import {
  MAX_CIPHERTEXT_SIZE,
  VERSION_V1,
  buildRciContext,
  decodeBase64UrlNoPad,
  decodeBase64UrlNoPadExact,
  encodeBase64UrlNoPad,
  encrypt,
} from ".";

const fixture = {
  node_private_key: "BwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwcHBwc",
  node_public_key: "E75P6uryBMf9M1j8nAByGIHRdCeBKCJ-xnTzf3_pe20",
  admin_private_key: "CQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQkJCQk",
  nonce: "CwsLCwsLCwsLCwsLCwsLCwsLCwsLCwsL",
  plaintext: "Bearer sk-rci-fixture",
  context: {
    node_id: "node-123",
    pending_credential_id: "pending-456",
    service_slug: "openai",
    injection_method: "header",
    field_name: "Authorization",
    target_url: "https://api.openai.com/v1",
    version: "v1" as const,
  },
  envelope: {
    version: "v1" as const,
    admin_pubkey: "V9tLNZ8jrl4Ubk4lEgVnBHIlBjSMFQwUdT0Mkz0E1CE",
    nonce: "CwsLCwsLCwsLCwsLCwsLCwsLCwsLCwsL",
    ciphertext: "9sHPUj2M3V_cDMib_f3PdlJK8RDep08jkBha8pBhLhP0b2SOHg",
  },
};

function fixtureContext() {
  return buildRciContext(fixture.context);
}

function deterministicEncrypt(
  plaintext: Uint8Array,
  nodePubkey: Uint8Array,
  adminPrivateKey: Uint8Array,
  nonce: Uint8Array,
) {
  const context = fixtureContext();
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

function testLocalDecrypt() {
  const context = fixtureContext();
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

describe("base64url no-pad codec", () => {
  it("encodes without padding and rejects padded input", () => {
    expect(encodeBase64UrlNoPad(Uint8Array.from([1, 2, 3]))).toBe("AQID");
    expect(() => decodeBase64UrlNoPad("AQID=", "value")).toThrow(
      "without padding",
    );
  });

  it("enforces exact public key and nonce lengths", () => {
    expect(
      decodeBase64UrlNoPadExact(fixture.node_public_key, "node_pubkey", 32),
    ).toHaveLength(32);
    expect(decodeBase64UrlNoPadExact(fixture.nonce, "nonce", 24)).toHaveLength(
      24,
    );
    expect(() => decodeBase64UrlNoPadExact("AQID", "admin_pubkey", 32)).toThrow(
      "32 bytes",
    );
  });
});

describe("RciContext byte vectors", () => {
  it("matches Rust kdf_info and aad vectors", () => {
    const context = fixtureContext();
    expect(encodeBase64UrlNoPad(context.kdfInfoBytes())).toBe(
      "bnl4aWQ6cmNpOnYxOmtkZgAACG5vZGUtMTIzAAtwZW5kaW5nLTQ1NgAGb3BlbmFpAAJ2MQ",
    );
    expect(encodeBase64UrlNoPad(context.aadBytes())).toBe(
      "bnl4aWQ6cmNpOnYxOmFhZAAACG5vZGUtMTIzAAtwZW5kaW5nLTQ1NgAGb3BlbmFpAAZoZWFkZXIADUF1dGhvcml6YXRpb24AGWh0dHBzOi8vYXBpLm9wZW5haS5jb20vdjEAAnYx",
    );
  });
});

describe("RCI v1 encryption", () => {
  it("decrypts the Rust fixture with a test-local helper", () => {
    expect(new TextDecoder().decode(testLocalDecrypt())).toBe(
      fixture.plaintext,
    );
  });

  it("matches the deterministic Rust fixture envelope", () => {
    const generated = deterministicEncrypt(
      new TextEncoder().encode(fixture.plaintext),
      decodeBase64UrlNoPadExact(fixture.node_public_key, "node_public_key", 32),
      decodeBase64UrlNoPadExact(
        fixture.admin_private_key,
        "admin_private_key",
        32,
      ),
      decodeBase64UrlNoPadExact(fixture.nonce, "nonce", 24),
    );

    expect(generated).toEqual(fixture.envelope);
  });

  it("uses a fresh nonce for production encrypt calls", () => {
    const context = fixtureContext();
    const nodePubkey = decodeBase64UrlNoPadExact(
      fixture.node_public_key,
      "node_public_key",
      32,
    );
    const first = encrypt(
      new TextEncoder().encode("same plaintext"),
      nodePubkey,
      context,
    );
    const second = encrypt(
      new TextEncoder().encode("same plaintext"),
      nodePubkey,
      context,
    );

    expect(first.nonce).not.toBe(second.nonce);
    expect(first.ciphertext).not.toBe(second.ciphertext);
  });

  it("rejects ciphertext above the cap", () => {
    const context = fixtureContext();
    const nodePubkey = decodeBase64UrlNoPadExact(
      fixture.node_public_key,
      "node_public_key",
      32,
    );
    expect(() =>
      encrypt(new Uint8Array(MAX_CIPHERTEXT_SIZE), nodePubkey, context),
    ).toThrow("ciphertext exceeds maximum size");
  });

  it("does not export decrypt from the production crypto barrel", async () => {
    const exports = await import(".");
    expect("decrypt" in exports).toBe(false);
  });
});
