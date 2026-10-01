import fs from "node:fs";
import path from "node:path";
import type { Plugin } from "vite";

/** The CLI resource is the only source; no generated public copy is committed. */
export function machineSeccomp(root: string): Plugin {
  const source = path.resolve(
    root,
    "../cli/resources/machine-container/seccomp.json",
  );
  const filename = "machine-seccomp.json";
  let building = false;
  return {
    name: "nyxid-machine-seccomp",
    configResolved(config) {
      building = config.command === "build";
    },
    buildStart() {
      this.addWatchFile(source);
      if (!building) return;
      this.emitFile({
        type: "asset",
        fileName: filename,
        source: fs.readFileSync(source),
      });
    },
    configureServer(server) {
      server.middlewares.use(`/${filename}`, (_request, response) => {
        response.setHeader("Content-Type", "application/json");
        response.end(fs.readFileSync(source));
      });
    },
    writeBundle(options) {
      // Includes publicDir overwrites: abort if the published profile diverges.
      const output = path.resolve(root, options.dir ?? "dist", filename);
      if (!fs.readFileSync(output).equals(fs.readFileSync(source))) {
        throw new Error(
          "Published machine seccomp profile differs from CLI source",
        );
      }
    },
  };
}
