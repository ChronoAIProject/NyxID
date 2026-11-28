const fs = require("node:fs/promises");
const path = require("node:path");

const artifacts = new Map([
  ["aab", ["android", ".aab"]],
  ["apk", ["android", ".apk"]],
  ["ipa", ["ios", ".ipa"]],
  ["simulator_app_uncompressed", ["ios", ".app"]],
]);

async function directoryBytes(directory) {
  let bytes = 0;
  for (const entry of await fs.readdir(directory, { withFileTypes: true })) {
    const fullPath = path.join(directory, entry.name);
    if (entry.isDirectory()) bytes += await directoryBytes(fullPath);
    if (entry.isFile()) bytes += (await fs.stat(fullPath)).size;
  }
  return bytes;
}

async function measureArtifact(kind, file) {
  const type = artifacts.get(kind);
  if (!type || path.extname(file).toLowerCase() !== type[1])
    throw new Error("Artifact kind and extension must match");
  const stat = await fs.lstat(file);
  if (kind === "simulator_app_uncompressed" && stat.isDirectory())
    return directoryBytes(file);
  if (kind !== "simulator_app_uncompressed" && stat.isFile()) return stat.size;
  throw new Error("Unexpected artifact file type");
}

module.exports = { artifacts, measureArtifact };
