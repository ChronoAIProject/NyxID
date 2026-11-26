import fs from "node:fs";
import path from "node:path";
import { execFileSync } from "node:child_process";
import { expect, it } from "vitest";
import { buildArtifactId, isCrashArtifactId } from "../scripts/crash-build-identity.mjs";

it("crash artifact identity accepts a supplied UUID rejects malformed input and generates a local fallback", () => {
  const supplied = "2E6E467D-54B7-4839-BFB1-C3E563368B37";
  expect(buildArtifactId(supplied)).toBe(supplied.toLowerCase());
  for (const bad of ["not-a-uuid", "-".repeat(36), "../private", `${supplied}\n`]) {
    expect(() => buildArtifactId(bad)).toThrow("Invalid crash artifact identity");
    expect(isCrashArtifactId(bad)).toBe(false);
  }
  const one = buildArtifactId(), two = buildArtifactId();
  expect(isCrashArtifactId(one)).toBe(true);
  expect(one).not.toBe(two);
});
it("publish workflow derives one deterministic identity for both architectures and uploads only once", () => {
  const root = path.resolve(import.meta.dirname, "../..");
  const workflow = fs.readFileSync(path.join(root, ".github/workflows/publish-images.yml"), "utf8");
  expect(workflow).toContain("needs: frontend-identity");
  expect(workflow).toContain("CRASH_ARTIFACT_ID=${{ needs.frontend-identity.outputs.artifact_id }}");
  expect(workflow).toContain("matrix.component == 'frontend' && matrix.platform == 'linux/amd64' && secrets.CRASH_ARTIFACTS_SENTRY");
  const script = path.join(root, ".github/scripts/crash-artifact-id.py");
  const environment = { ...process.env, GITHUB_REPOSITORY: "ChronoAIProject/NyxID", GITHUB_SHA: "abc123", GITHUB_RUN_ID: "1234" };
  const identity = () => execFileSync("python3", [script], { env: environment, encoding: "utf8" }).trim();
  const one = identity();
  expect(identity()).toBe(one);
  expect(isCrashArtifactId(one)).toBe(true);
  environment.GITHUB_RUN_ID = "1235";
  expect(identity()).not.toBe(one);
  const dockerfile = fs.readFileSync(path.join(root, "frontend/Dockerfile"), "utf8");
  expect(dockerfile).toContain("ARG CRASH_ARTIFACT_ID");
  expect(dockerfile).toContain("ENV CRASH_ARTIFACT_ID=${CRASH_ARTIFACT_ID}");
  const vite = fs.readFileSync(path.join(root, "frontend/vite.config.ts"), "utf8");
  expect(vite).toContain("buildArtifactId(process.env.CRASH_ARTIFACT_ID)");
});
