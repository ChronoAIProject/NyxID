import { createHash } from "node:crypto";
import { mkdtemp, readFile, rm } from "node:fs/promises";
import os from "node:os";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { build } from "vite";
import { describe, expect, it } from "vitest";
import {
  CREDENTIAL_ACCEPT_SCRIPT_ROLE,
  credentialAcceptFingerprintSha384Hex,
  RELEASE_INTEGRITY_SCHEMA_VERSION,
  type CredentialAcceptScriptBytes,
} from "@/lib/release-integrity/manifest";
import {
  buildReleaseIntegrityManifest,
  credentialAcceptFingerprintSha384HexNode,
} from "../vite-plugins/release-integrity";

const encoder = new TextEncoder();
const frontendRoot = path.resolve(path.dirname(fileURLToPath(import.meta.url)), "..");

function sha384(bytes: Uint8Array): { sri: string; hex: string } {
  const digest = createHash("sha384").update(bytes).digest();
  return {
    sri: `sha384-${digest.toString("base64")}`,
    hex: digest.toString("hex"),
  };
}

describe("release integrity manifest", () => {
  it("eval_sri_hash_format: HTML SRI, manifest digests, and page fingerprint use the same bytes", async () => {
    const scripts: CredentialAcceptScriptBytes[] = [
      {
        path: "/credential-accept/assets/credential-accept-b.js",
        bytes: encoder.encode("globalThis.b = 2;"),
      },
      {
        path: "/credential-accept/assets/credential-accept-a.js",
        bytes: encoder.encode("globalThis.a = 1;"),
      },
    ];
    const scriptHashes = new Map(
      scripts.map((script) => [script.path, sha384(script.bytes)]),
    );
    const html = `<!doctype html><html><head></head><body>${scripts
      .map((script) => {
        const hash = scriptHashes.get(script.path);
        return `<script type="module" data-nyx-integrity-role="credential_accept_script" src="${script.path}" integrity="${hash?.sri}" crossorigin="anonymous"></script>`;
      })
      .join("")}</body></html>`;
    const htmlBytes = encoder.encode(html);
    const manifest = buildReleaseIntegrityManifest({
      htmlPath: "/credential-accept/credential-accept.html",
      htmlBytes,
      scripts,
      appVersion: "0.6.0-test",
      gitCommit: "abc123",
      generatedAt: "2026-06-05T00:00:00.000Z",
    });

    expect(manifest.schema_version).toBe(RELEASE_INTEGRITY_SCHEMA_VERSION);
    for (const script of scripts) {
      const htmlSri = html.match(
        new RegExp(`src="${script.path}" integrity="([^"]+)"`),
      )?.[1];
      const artifact = manifest.artifacts.find(
        (entry) => entry.path === script.path,
      );
      const scriptHash = scriptHashes.get(script.path);
      expect(artifact).toBeDefined();
      expect(htmlSri).toBe(artifact?.sha384_sri);
      expect(artifact?.sha384_sri).toBe(scriptHash?.sri);
      expect(artifact?.sha384_hex).toBe(scriptHash?.hex);
      expect(artifact?.sha384_sri).toMatch(/^sha384-[A-Za-z0-9+/]+={0,2}$/);
      expect(artifact?.sha384_hex).toMatch(/^[0-9a-f]{96}$/);
    }

    expect(manifest.credential_accept.fingerprint_sha384_hex).toBe(
      credentialAcceptFingerprintSha384HexNode(scripts),
    );
    await expect(credentialAcceptFingerprintSha384Hex(scripts)).resolves.toBe(
      manifest.credential_accept.fingerprint_sha384_hex,
    );
  });

  it("eval_build_output_metadata_role: emitted credential accept scripts keep SRI and scanner metadata", async () => {
    const tmp = await mkdtemp(path.join(os.tmpdir(), "nyxid-credential-accept-"));
    const outDir = path.join(tmp, "credential-accept");
    try {
      await build({
        configFile: path.join(frontendRoot, "vite.credential-accept.config.ts"),
        logLevel: "silent",
        build: {
          outDir,
          emptyOutDir: true,
        },
      });

      const html = await readFile(path.join(outDir, "credential-accept.html"), "utf8");
      const doc = new DOMParser().parseFromString(html, "text/html");
      const credentialScripts = Array.from(
        doc.querySelectorAll<HTMLScriptElement>(
          'script[src^="/credential-accept/assets/"]',
        ),
      );
      expect(credentialScripts.length).toBeGreaterThan(0);
      for (const script of credentialScripts) {
        expect(script.getAttribute("integrity")).toMatch(
          /^sha384-[A-Za-z0-9+/]+={0,2}$/,
        );
        expect(script.getAttribute("data-nyx-integrity-role")).toBe(
          CREDENTIAL_ACCEPT_SCRIPT_ROLE,
        );
      }
      expect(
        doc.querySelectorAll<HTMLScriptElement>(
          'script[data-nyx-integrity-role="credential_accept_script"][src]',
        ),
      ).toHaveLength(credentialScripts.length);
    } finally {
      await rm(tmp, { recursive: true, force: true });
    }
  }, 60_000);
});
