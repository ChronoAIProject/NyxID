import { readFile } from "node:fs/promises";
import { createHash } from "node:crypto";
import { fileURLToPath } from "node:url";
import path from "node:path";
const directory = fileURLToPath(
  new URL("../../shared/agent-avatars/rive/", import.meta.url),
);
const hash = (bytes) => createHash("sha256").update(bytes).digest("hex");
export async function emoteSourceHash() {
  const parts = await Promise.all(
    ["rive.yaml", "scene.rml", "prop-art.json", "generate-props.mjs"].map(
      async (name) =>
        `${name}\0${await readFile(path.join(directory, name), "utf8")}\0`,
    ),
  );
  return hash(parts.join(""));
}
export async function checkEmoteAsset() {
  const manifest = JSON.parse(
    await readFile(path.join(directory, "manifest.json"), "utf8"),
  );
  const bytes = await readFile(path.join(directory, "nyx-emotes.riv"));
  if (
    bytes.subarray(0, 4).toString() !== "RIVE" ||
    hash(bytes) !== manifest.assetSha256 ||
    (await emoteSourceHash()) !== manifest.sourceSha256
  )
    throw Error(
      "Rive emotes are stale. Rebuild the script-free local asset with build-avatar-emotes.mjs.",
    );
  return manifest;
}
if (process.argv[1] === fileURLToPath(import.meta.url)) {
  await checkEmoteAsset();
  console.log("Rive emote source/asset freshness passed.");
}
