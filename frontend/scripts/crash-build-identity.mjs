import { randomUUID } from "node:crypto";

export function isCrashArtifactId(value) {
  return typeof value === "string" &&
    /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i.test(value);
}

/** CI supplies one identity to both architectures; local builds remain unique. */
export function buildArtifactId(provided) {
  if (provided === undefined || provided === "") return randomUUID();
  if (!isCrashArtifactId(provided)) throw new Error("Invalid crash artifact identity");
  return provided.toLowerCase();
}
