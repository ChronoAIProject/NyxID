import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";

const frontend = fileURLToPath(new URL("../", import.meta.url));
const port = process.env.NYXID_PREVIEW_PORT || "4321";
const backend = process.env.BACKEND_URL || "https://nyxid.ai";
const origin = process.env.FRONTEND_URL || "https://nyxid.ai";

console.info(`NyxID frontend: http://127.0.0.1:${port}/assistant`);
console.info(`API backend: ${new URL(backend).origin}`);

const child = spawn(
  process.execPath,
  ["node_modules/vite/bin/vite.js", "--host", "127.0.0.1", "--port", port, "--strictPort"],
  {
    cwd: frontend,
    stdio: "inherit",
    env: {
      ...process.env,
      BACKEND_URL: backend,
      FRONTEND_URL: origin,
      NODE_USE_ENV_PROXY: process.env.NODE_USE_ENV_PROXY || "1",
    },
  },
);

for (const signal of ["SIGINT", "SIGTERM"]) {
  process.on(signal, () => child.kill(signal));
}
child.on("error", (error) => {
  console.error(error.message);
  process.exitCode = 1;
});
child.on("exit", (code, signal) => {
  process.exitCode = code ?? (signal === "SIGINT" ? 130 : 1);
});
