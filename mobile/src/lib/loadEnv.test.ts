import assert from "node:assert/strict";
import { createRequire } from "node:module";
import test from "node:test";

type ResolvedProfile = {
  frontendUrl: string;
};

type ResolveProfile = (
  appEnv: "dev" | "prod",
  env: Record<string, string>,
  options?: { warnOnFrontendUrlFallback?: boolean }
) => ResolvedProfile;

const require = createRequire(import.meta.url);
const { resolveProfile } = require("../../scripts/lib/load-env.js") as {
  resolveProfile: ResolveProfile;
};

function profileEnv(overrides: Record<string, string> = {}): Record<string, string> {
  return {
    DEV_API_BASE_URL: "http://localhost:3001/api/v1",
    DEV_IOS_BUNDLE_ID: "dev.nyxid.test",
    DEV_ANDROID_PACKAGE: "dev.nyxid.test",
    ...overrides,
  };
}

function captureWarnings(run: () => void): string[] {
  const originalWarn = console.warn;
  const warnings: string[] = [];
  console.warn = (...values: unknown[]) => warnings.push(values.map(String).join(" "));
  try {
    run();
  } finally {
    console.warn = originalWarn;
  }
  return warnings;
}

test("does not warn when the frontend URL is configured explicitly", () => {
  const warnings = captureWarnings(() => {
    const resolved = resolveProfile(
      "dev",
      profileEnv({ DEV_FRONTEND_URL: "http://localhost:3000" }),
      { warnOnFrontendUrlFallback: true }
    );
    assert.equal(resolved.frontendUrl, "http://localhost:3000");
  });

  assert.deepEqual(warnings, []);
});

test("warns when the trusted frontend origin falls back to the legal URL", () => {
  const warnings = captureWarnings(() => {
    const resolved = resolveProfile(
      "dev",
      profileEnv({ DEV_LEGAL_BASE_URL: "http://legal.localhost:3000" }),
      { warnOnFrontendUrlFallback: true }
    );
    assert.equal(resolved.frontendUrl, "http://legal.localhost:3000");
  });

  assert.equal(warnings.length, 1);
  assert.match(warnings[0] ?? "", /DEV_FRONTEND_URL is not explicitly configured/);
  assert.match(warnings[0] ?? "", /moving legal documents cannot break QR scanning/);
});

test("warns loudly when no trusted frontend URL resolves", () => {
  const warnings = captureWarnings(() => {
    const resolved = resolveProfile("dev", profileEnv(), {
      warnOnFrontendUrlFallback: true,
    });
    assert.equal(resolved.frontendUrl, "");
  });

  assert.equal(warnings.length, 1);
  assert.match(warnings[0] ?? "", /NO TRUSTED FRONTEND URL RESOLVED FOR DEV/);
  assert.match(warnings[0] ?? "", /Web QR login scans will be rejected/);
});

test("keeps non-build profile resolution quiet", () => {
  const warnings = captureWarnings(() => {
    const resolved = resolveProfile(
      "dev",
      profileEnv({ DEV_LEGAL_BASE_URL: "http://legal.localhost:3000" })
    );
    assert.equal(resolved.frontendUrl, "http://legal.localhost:3000");
  });

  assert.deepEqual(warnings, []);
});
