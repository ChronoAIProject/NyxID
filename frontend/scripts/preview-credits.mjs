import { spawn } from "node:child_process";
import { fileURLToPath } from "node:url";
import path from "node:path";

const frontendRoot = fileURLToPath(new URL("../", import.meta.url));

function origin(value, name) {
  const url = new URL(value);
  if (!['http:', 'https:'].includes(url.protocol) || url.username || url.password ||
      url.search || url.hash || url.pathname !== '/') {
    throw new Error(`${name} must be an HTTP(S) origin without credentials, path, query, or fragment`);
  }
  return url.origin;
}

export function previewOptions(args, env = process.env) {
  let host = "127.0.0.1";
  let port = 3000;
  let checkOnly = false;
  for (let i = 0; i < args.length; i++) {
    const argument = args[i];
    if (argument === "--check") checkOnly = true;
    else if (argument === "--strictPort") continue;
    else if (argument === "--host") host = args[++i];
    else if (argument === "--port") port = Number(args[++i]);
    else throw new Error(`Unsupported preview option: ${argument}`);
  }
  if (!['127.0.0.1', 'localhost'].includes(host) || !Number.isInteger(port) || port < 1 || port > 65535) {
    throw new Error("Preview requires a loopback host (127.0.0.1 or localhost) and a valid port");
  }
  const localOrigin = `http://${host}:${port}`;
  return {
    host, port, checkOnly, localOrigin,
    backend: origin(env.BACKEND_URL || "http://localhost:3001", "BACKEND_URL"),
    expectedOrigin: origin(env.FRONTEND_URL || localOrigin, "FRONTEND_URL"),
  };
}

/** Read-only configuration checks. Passing does not claim authenticated acceptance. */
export async function checkPreview(options, request = fetch) {
  const failures = [];
  const checks = [];
  async function get(route) {
    try {
      return await request(`${options.backend}${route}`, {
        redirect: "manual", signal: AbortSignal.timeout(10_000),
        headers: { Accept: "application/json" },
      });
    } catch {
      failures.push(`${route}: backend unreachable or request timed out`);
      return null;
    }
  }
  async function json(route) {
    const response = await get(route);
    if (!response) return null;
    let body;
    try { body = await response.json(); } catch { /* HTML redirects are not API readiness. */ }
    if (response.status !== 200 || !body || typeof body !== "object" || Array.isArray(body)) {
      failures.push(`${route}: expected JSON HTTP 200, received HTTP ${response.status}`);
      return null;
    }
    return body;
  }
  const health = await json("/health");
  if (health && health.status !== "ok") failures.push("/health: backend is not healthy");
  if (health?.status === "ok") checks.push("Backend health responds with JSON");
  const config = await json("/api/v1/public/config");

  for (const route of ["/api/v1/admin/credits/summary", "/api/v1/billing/credits"]) {
    const response = await get(route);
    if (!response) continue;
    let body;
    try { body = await response.json(); } catch { /* Reject SPA fallbacks and empty errors. */ }
    // Both routes require auth. An anonymous 200 is also a broken contract.
    if (response.status === 401 && body?.error === "unauthorized" && body?.error_code === 1001) {
      checks.push(`${route}: authenticated route is present`);
    } else {
      failures.push(`${route}: expected the protected Credits API, received HTTP ${response.status}. Deploy the matching backend before previewing this frontend.`);
    }
  }

  if (config) {
    const providers = config.social_providers;
    if (!Array.isArray(providers) || !providers.every(p => ['google', 'github', 'apple'].includes(p)) ||
        typeof config.email_auth_enabled !== "boolean") {
      failures.push("/api/v1/public/config: invalid sign-in configuration");
    } else {
      if (!config.email_auth_enabled && providers.length === 0) {
        failures.push("No email or social sign-in is configured; a device-only login needs separate authenticated verification before previewing.");
      }
      for (const provider of providers) {
        const returnTo = `${options.localOrigin}/admin/credits`;
        const route = `/api/v1/auth/social/${provider}?return_to=${encodeURIComponent(returnTo)}`;
        const response = await get(route);
        if (!response) continue;
        let callback;
        try {
          const location = new URL(response.headers.get("location"));
          callback = new URL(location.searchParams.get("redirect_uri"));
        } catch { /* No valid provider callback. */ }
        if (![302, 303, 307].includes(response.status) || !callback ||
            callback.hostname !== new URL(options.localOrigin).hostname) {
          failures.push(`${provider} login: the provider callback does not return to the local cookie host. A remote social-login callback cannot use localhost's sign-in cookie.`);
        }
        const cookies = response.headers.getSetCookie();
        const validReturn = cookies.some(cookie => {
          if (!cookie.startsWith("nyx_social_return_to=") || /;\s*Max-Age=0(?:;|$)/i.test(cookie)) return false;
          try { return decodeURIComponent(cookie.split(";", 1)[0].slice("nyx_social_return_to=".length)) === returnTo; }
          catch { return false; }
        });
        if (!validReturn) failures.push(`${provider} login: backend rejected the local return URL ${returnTo}`);
      }
      if (config.email_auth_enabled) checks.push("Email sign-in is enabled; verify its session and CSRF through the local proxy");
    }
  }
  return { checks, failures };
}

async function run(command, args, env) {
  const child = spawn(command, args, { cwd: frontendRoot, env, stdio: "inherit" });
  const forward = signal => child.kill(signal);
  const interrupt = () => forward("SIGINT");
  const terminate = () => forward("SIGTERM");
  process.on("SIGINT", interrupt);
  process.on("SIGTERM", terminate);
  try {
    await new Promise((resolve, reject) => {
      child.once("error", reject);
      child.once("exit", (code, signal) => code === 0 ? resolve() : reject(new Error(`Preview command stopped (${signal || code})`)));
    });
  } finally {
    process.off("SIGINT", interrupt);
    process.off("SIGTERM", terminate);
  }
}

async function main() {
  const options = previewOptions(process.argv.slice(2));
  console.log(`Checking ${options.backend} for preview at ${options.localOrigin}`);
  const result = await checkPreview(options);
  for (const check of result.checks) console.log(`PASS ${check}`);
  if (result.failures.length) {
    for (const failure of result.failures) console.error(`BLOCKED ${failure}`);
    throw new Error("Preview was not started. Resolve the backend and login mismatches first.");
  }
  console.log("Configuration checks passed. Authenticated browser verification is still required before sharing the preview.");
  if (options.checkOnly) return;
  const env = { ...process.env, BACKEND_URL: options.backend, FRONTEND_URL: options.expectedOrigin };
  await run(process.platform === "win32" ? "npm.cmd" : "npm", ["run", "build"], env);
  await run(process.execPath, [path.join(frontendRoot, "node_modules/vite/bin/vite.js"), "preview", "--host", options.host, "--port", String(options.port), "--strictPort"], env);
}

if (process.argv[1] && path.resolve(process.argv[1]) === fileURLToPath(import.meta.url)) {
  main().catch(error => { console.error(error.message); process.exitCode = 1; });
}
