import { createHash } from "node:crypto";
import fs from "node:fs";
import path from "node:path";
import type { OutputAsset, OutputBundle, OutputChunk } from "rollup";
import type { Plugin } from "vite";
import {
  CREDENTIAL_ACCEPT_FINGERPRINT_PREFIX,
  CREDENTIAL_ACCEPT_HTML_ROLE,
  CREDENTIAL_ACCEPT_SCRIPT_ROLE,
  RELEASE_INTEGRITY_SCHEMA_VERSION,
  type ReleaseIntegrityArtifact,
  type ReleaseIntegrityManifest,
} from "../src/lib/release-integrity/manifest";

interface ScriptArtifactBytes {
  readonly path: string;
  readonly bytes: Uint8Array;
}

interface IntegrityHash {
  readonly sha384_sri: string;
  readonly sha384_hex: string;
}

const SCRIPT_TAG_RE = /<script\b([^>]*?)\bsrc="([^"]+)"([^>]*)><\/script>/g;
const SCRIPT_ROLE_ATTR = `data-nyx-integrity-role="${CREDENTIAL_ACCEPT_SCRIPT_ROLE}"`;

function bytesFromSource(source: string | Uint8Array): Uint8Array {
  return typeof source === "string" ? new TextEncoder().encode(source) : source;
}

function sha384(bytes: Uint8Array): IntegrityHash {
  const digest = createHash("sha384").update(bytes).digest();
  return {
    sha384_sri: `sha384-${digest.toString("base64")}`,
    sha384_hex: digest.toString("hex"),
  };
}

function u32be(value: number): Buffer {
  const out = Buffer.alloc(4);
  out.writeUInt32BE(value);
  return out;
}

function u64be(value: number): Buffer {
  const out = Buffer.alloc(8);
  out.writeBigUInt64BE(BigInt(value));
  return out;
}

function compareUtf8PathBytes(a: string, b: string): number {
  return Buffer.compare(Buffer.from(a, "utf8"), Buffer.from(b, "utf8"));
}

export function credentialAcceptFingerprintSha384HexNode(
  scripts: readonly ScriptArtifactBytes[],
): string {
  const hash = createHash("sha384");
  hash.update(Buffer.from(CREDENTIAL_ACCEPT_FINGERPRINT_PREFIX, "utf8"));
  const sorted = [...scripts].sort((a, b) => compareUtf8PathBytes(a.path, b.path));
  for (const script of sorted) {
    const pathBytes = Buffer.from(script.path, "utf8");
    hash.update(u32be(pathBytes.length));
    hash.update(pathBytes);
    hash.update(u64be(script.bytes.length));
    hash.update(script.bytes);
  }
  return hash.digest("hex");
}

function publicPathForFile(fileName: string): string {
  return `/credential-accept/${fileName.replace(/\\/g, "/")}`;
}

function contentTypeForPath(filePath: string): string {
  if (filePath.endsWith(".html")) return "text/html; charset=utf-8";
  if (filePath.endsWith(".js")) return "text/javascript; charset=utf-8";
  return "application/octet-stream";
}

function artifact(
  role: ReleaseIntegrityArtifact["role"],
  artifactPath: string,
  bytes: Uint8Array,
): ReleaseIntegrityArtifact {
  return {
    role,
    path: artifactPath,
    content_type: contentTypeForPath(artifactPath),
    size_bytes: bytes.length,
    ...sha384(bytes),
  };
}

function sourceForBundleEntry(entry: OutputAsset | OutputChunk): Uint8Array {
  if (entry.type === "chunk") {
    return bytesFromSource(entry.code);
  }
  if (typeof entry.source === "string") {
    return bytesFromSource(entry.source);
  }
  return new Uint8Array(entry.source);
}

export function buildReleaseIntegrityManifest(params: {
  htmlPath: string;
  htmlBytes: Uint8Array;
  scripts: readonly ScriptArtifactBytes[];
  appVersion: string;
  gitCommit: string;
  generatedAt: string;
}): ReleaseIntegrityManifest {
  return {
    schema_version: RELEASE_INTEGRITY_SCHEMA_VERSION,
    app_version: params.appVersion,
    git_commit: params.gitCommit,
    generated_at: params.generatedAt,
    credential_accept: {
      fingerprint_sha384_hex: credentialAcceptFingerprintSha384HexNode(params.scripts),
    },
    artifacts: [
      artifact(CREDENTIAL_ACCEPT_HTML_ROLE, params.htmlPath, params.htmlBytes),
      ...params.scripts.map((script) =>
        artifact(CREDENTIAL_ACCEPT_SCRIPT_ROLE, script.path, script.bytes),
      ),
    ],
  };
}

function injectSriIntoHtml(html: string, scripts: Map<string, IntegrityHash>): string {
  let seen = 0;
  return html.replace(SCRIPT_TAG_RE, (full, before: string, src: string, after: string) => {
    const hash = scripts.get(src);
    if (!hash) return full;
    seen += 1;
    const attrs = `${before} src="${src}"${after}`
      .replace(/\s+integrity="[^"]*"/, "")
      .replace(/\s+crossorigin(?:="[^"]*")?/, "")
      .replace(/\s+data-nyx-integrity-role="[^"]*"/, "");
    return `<script${attrs} ${SCRIPT_ROLE_ATTR} integrity="${hash.sha384_sri}" crossorigin="anonymous"></script>`;
  }).replace(/<\/head>/, `<meta name="nyx-release-integrity-scripts" content="${String(seen)}"></head>`);
}

function assertHtmlScriptsCovered(html: string, expectedScripts: Iterable<string>): string[] {
  const uncovered = new Set<string>();
  const covered = new Set<string>();
  for (const match of html.matchAll(SCRIPT_TAG_RE)) {
    const tag = match[0] ?? "";
    const src = match[2] ?? "";
    if (!src.startsWith("/credential-accept/assets/")) continue;
    covered.add(src);
    if (!/\sintegrity="sha384-[A-Za-z0-9+/]+={0,2}"/.test(tag)) {
      uncovered.add(`${src} missing SRI`);
    }
    if (!/\scrossorigin="anonymous"/.test(tag)) {
      uncovered.add(`${src} missing crossorigin`);
    }
    if (!tag.includes(` ${SCRIPT_ROLE_ATTR}`)) {
      uncovered.add(`${src} missing integrity role`);
    }
  }
  for (const src of expectedScripts) {
    if (!covered.has(src)) {
      uncovered.add(`${src} missing script tag`);
    }
  }
  return [...uncovered];
}

export function releaseIntegrityPlugin(): Plugin {
  return {
    name: "nyxid-release-integrity",
    enforce: "post",
    generateBundle(_options, bundle: OutputBundle) {
      const htmlEntry = Object.entries(bundle).find(
        ([fileName, entry]) => entry.type === "asset" && fileName.endsWith(".html"),
      );
      if (!htmlEntry) {
        this.error("credential accept HTML asset was not emitted");
        return;
      }

      const [htmlFileName, htmlAsset] = htmlEntry as [string, OutputAsset];
      let html = String(htmlAsset.source);
      const scripts = new Map<string, IntegrityHash>();
      const scriptBytes: ScriptArtifactBytes[] = [];

      for (const [fileName, entry] of Object.entries(bundle)) {
        if (entry.type !== "chunk" || !fileName.endsWith(".js")) continue;
        const bytes = sourceForBundleEntry(entry);
        const publicPath = publicPathForFile(fileName);
        scripts.set(publicPath, sha384(bytes));
        scriptBytes.push({ path: publicPath, bytes });
      }

      if (scriptBytes.length === 0) {
        this.error("credential accept build emitted no script chunks");
        return;
      }

      html = injectSriIntoHtml(html, scripts);
      const uncovered = assertHtmlScriptsCovered(html, scripts.keys());
      if (uncovered.length > 0) {
        this.error(
          `credential accept HTML has script tags without required integrity metadata: ${uncovered.join(", ")}`,
        );
        return;
      }

      htmlAsset.source = html;
      const htmlBytes = bytesFromSource(html);
      const manifest = buildReleaseIntegrityManifest({
        htmlPath: publicPathForFile(htmlFileName),
        htmlBytes,
        scripts: scriptBytes,
        appVersion: process.env.npm_package_version ?? "0.0.0",
        gitCommit:
          process.env.GITHUB_SHA ??
          process.env.NYXID_GIT_COMMIT ??
          process.env.VITE_GIT_COMMIT ??
          "unknown",
        generatedAt: new Date().toISOString(),
      });

      this.emitFile({
        type: "asset",
        fileName: "release-integrity-manifest-placeholder.json",
        source: JSON.stringify(manifest, null, 2),
      });
    },
    writeBundle(options) {
      const outDir = options.dir;
      if (!outDir) return;
      const placeholder = path.join(outDir, "release-integrity-manifest-placeholder.json");
      const releasesDir = path.resolve(outDir, "../release-integrity");
      const releasesPath = path.join(releasesDir, "releases.json");
      fs.mkdirSync(releasesDir, { recursive: true });
      fs.copyFileSync(placeholder, releasesPath);
      fs.rmSync(placeholder, { force: true });
    },
  };
}
