import { readFile, readdir, stat } from "node:fs/promises";
import { fileURLToPath } from "node:url";
import path from "node:path";

const frontendRoot = fileURLToPath(new URL("../", import.meta.url));
const distRoot = path.join(frontendRoot, "dist");
const credentialAcceptRoot = path.join(distRoot, "credential-accept");
const forbiddenSymbols = ["mockchat-", "scenario-engine", "mockscenarios"];

async function filesBelow(directory) {
  const entries = await readdir(directory, { withFileTypes: true });
  const files = [];
  for (const entry of entries) {
    const entryPath = path.join(directory, entry.name);
    if (entry.isDirectory()) {
      files.push(...(await filesBelow(entryPath)));
    } else if (entry.isFile()) {
      files.push(entryPath);
    }
  }
  return files;
}

let credentialAcceptStat;
try {
  credentialAcceptStat = await stat(credentialAcceptRoot);
} catch {
  throw new Error(
    "Production build is missing dist/credential-accept; the full build chain did not run.",
  );
}
if (!credentialAcceptStat.isDirectory()) {
  throw new Error("dist/credential-accept exists but is not a directory.");
}

const violations = [];
for (const file of await filesBelow(distRoot)) {
  const contents = await readFile(file);
  for (const symbol of forbiddenSymbols) {
    if (contents.includes(Buffer.from(symbol))) {
      violations.push(`${path.relative(distRoot, file)} contains ${symbol}`);
    }
  }
}

if (violations.length > 0) {
  throw new Error(
    `Production build contains dev-only mock scenario code:\n${violations.join("\n")}`,
  );
}

process.stdout.write(
  "Mock scenario footprint assertion passed; credential-accept output is present.\n",
);
