// @vitest-environment node
import { afterEach, expect, it } from "vitest";
import { build } from "vite";
import fs from "node:fs";
import os from "node:os";
import path from "node:path";
import { machineSeccomp } from "../scripts/machine-seccomp";

const directories: string[] = [];
afterEach(() =>
  directories
    .splice(0)
    .forEach((dir) => fs.rmSync(dir, { recursive: true, force: true })),
);

function fixture() {
  const directory = fs.mkdtempSync(path.join(os.tmpdir(), "nyxid-seccomp-"));
  directories.push(directory);
  const root = path.join(directory, "frontend");
  const source = path.join(
    directory,
    "cli/resources/machine-container/seccomp.json",
  );
  fs.mkdirSync(root, { recursive: true });
  fs.mkdirSync(path.dirname(source), { recursive: true });
  fs.copyFileSync(
    new URL(
      "../../cli/resources/machine-container/seccomp.json",
      import.meta.url,
    ),
    source,
  );
  fs.writeFileSync(path.join(root, "entry.js"), "export const ready = true;");
  return { root, source };
}

it("publishes exactly the CLI profile without a separate public source", async () => {
  const { root, source } = fixture();
  await build({
    configFile: false,
    root,
    logLevel: "silent",
    plugins: [machineSeccomp(root)],
    build: { rollupOptions: { input: path.join(root, "entry.js") } },
  });
  expect(fs.readFileSync(path.join(root, "dist/machine-seccomp.json"))).toEqual(
    fs.readFileSync(source),
  );
  expect(
    fs.existsSync(new URL("../public/machine-seccomp.json", import.meta.url)),
  ).toBe(false);
});

it("fails the build if a later plugin changes the published profile", async () => {
  const { root } = fixture();
  await expect(
    build({
      configFile: false,
      root,
      logLevel: "silent",
      plugins: [
        machineSeccomp(root),
        {
          name: "corrupt-profile",
          generateBundle(_options, bundle) {
            const asset = bundle["machine-seccomp.json"];
            if (asset?.type === "asset") asset.source = "{}";
          },
        },
      ],
      build: { rollupOptions: { input: path.join(root, "entry.js") } },
    }),
  ).rejects.toThrow("differs from CLI source");
});
