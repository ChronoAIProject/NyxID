/** Exercise the built worker under the exact separate SPA/worker CSPs. */
import { createServer } from "node:http";
import { readFile, readdir } from "node:fs/promises";
import { spawnSync } from "node:child_process";
import path from "node:path";
import { fileURLToPath } from "node:url";
import { chromium } from "@playwright/test";
const root = fileURLToPath(new URL("../", import.meta.url));
const dist = path.join(root, "dist");
const worker = (await readdir(path.join(dist, "assets"))).find((name) =>
  /^rive-motion\.worker-[\w-]+\.js$/.test(name),
);
if (!worker) throw Error("Build the frontend before verifying the Rive worker");
const generated = spawnSync(
  "python3",
  [
    "-c",
    `import json,runpy,pathlib; p=pathlib.Path('scripts'); m=runpy.run_path(str(p/'render-nginx.py')); print(json.dumps(m['render']({},(p.parent/'nginx.conf.template').read_text(),pathlib.Path('/tmp/nyx-rive-policy'),pathlib.Path('/tmp/dist'),pathlib.Path('/tmp/legal'),8080)))`,
  ],
  { cwd: root, encoding: "utf8" },
);
if (generated.status !== 0) throw Error(generated.stderr);
const policy = JSON.parse(generated.stdout);
const csp = (name) => /Content-Security-Policy "([^"]+)"/.exec(policy[name])[1];
let rigFailuresRemaining = 1;
const server = createServer(async (req, res) => {
  const pathname = new URL(req.url, "http://localhost").pathname;
  res.setHeader(
    "Content-Security-Policy",
    csp(
      pathname === `/assets/${worker}`
        ? "rive-worker-headers.conf"
        : "spa-headers.conf",
    ),
  );
  if (pathname === "/") {
    res.setHeader("Content-Type", "text/html");
    res.end('<!doctype html><script type="module" src="/probe.js"></script>');
    return;
  }
  if (pathname === "/probe.js") {
    res.setHeader("Content-Type", "text/javascript");
    res.end(
      `window.results=[];const worker=new Worker('/assets/${worker}',{type:'module'});window.probe=worker;worker.onmessage=event=>window.results.push(event.data);worker.postMessage({type:'add',id:1,emote:1});worker.postMessage({type:'state',id:1,emote:5});worker.postMessage({type:'active',id:1,active:false});worker.postMessage({type:'add',id:2,emote:1});worker.postMessage({type:'remove',id:2});`,
    );
    return;
  }
  if (!/^\/assets\/[\w.-]+$/.test(pathname)) {
    res.writeHead(404).end();
    return;
  }
  if (pathname.endsWith(".riv") && rigFailuresRemaining > 0) {
    rigFailuresRemaining -= 1;
    res.writeHead(404).end();
    return;
  }
  try {
    res.setHeader(
      "Content-Type",
      pathname.endsWith(".wasm")
        ? "application/wasm"
        : pathname.endsWith(".js")
          ? "text/javascript"
          : "application/octet-stream",
    );
    res.end(await readFile(path.join(dist, pathname)));
  } catch {
    res.writeHead(404).end();
  }
});
await new Promise((resolve) => server.listen(0, "127.0.0.1", resolve));
const browser = await chromium.launch();
try {
  const page = await browser.newPage();
  const errors = [];
  page.on("pageerror", (error) => errors.push(error.message));
  await page.goto(`http://127.0.0.1:${server.address().port}/`);
  await page.waitForFunction(() =>
    window.results?.some(
      (message) => message.type === "failed" && message.id === 1,
    ),
  );
  // Use the same worker so a rejected cached loading promise cannot pass by restart.
  await page.evaluate(() => {
    window.probe.postMessage({ type: "remove", id: 1 });
    window.probe.postMessage({ type: "add", id: 3, emote: 1, active: false });
  });
  await page.waitForFunction(() =>
    window.results?.some(
      (message) =>
        message.id === 3 &&
        (message.type === "ready" || message.type === "failed"),
    ),
  );
  const recovery = await page.evaluate(() => window.results);
  if (recovery.some((message) => message.id === 3 && message.type === "failed"))
    throw Error(
      `Rive worker did not recover after the transient rig 404: ${JSON.stringify(recovery)}`,
    );
  await page.evaluate(() => window.probe.terminate());
  await page.reload();
  await page.waitForFunction(() =>
    window.results?.some(
      (message) => message.type === "ready" || message.type === "failed",
    ),
  );
  await page.waitForTimeout(100);
  const paused = await page.evaluate(() => window.results);
  if (paused.some((message) => message.type === "poses" || message.id === 2))
    throw Error("A hidden or cancelled pending scene advanced during load");
  await page.evaluate(() =>
    window.probe.postMessage({ type: "active", id: 1, active: true }),
  );
  await page.waitForFunction(() =>
    window.results?.some(
      (message) =>
        message.type === "poses" &&
        message.poses.some(({ pose }) => pose.openness > 1.1),
    ),
  );
  const results = await page.evaluate(() => window.results);
  if (errors.length || results.some((message) => message.type === "failed"))
    throw Error(JSON.stringify({ errors, results }));
  await page.evaluate(() =>
    window.probe.postMessage({ type: "state", id: 1, emote: 1 }),
  );
  await page.waitForFunction(() =>
    window.results.some(
      (message) =>
        message.type === "poses" &&
        message.poses.some(({ pose }) => pose.y < -0.5),
    ),
  );
  const response = await page.request.get(
    `http://127.0.0.1:${server.address().port}/`,
  );
  if (
    response.headers()["content-security-policy"].includes("wasm-unsafe-eval")
  )
    throw Error("SPA CSP was widened");
  console.log(
    "Built Rive worker passed transient rig 404 recovery in the same worker, queued state/cancellation, pause/resume and interruption under its isolated WASM CSP; SPA policy stays unchanged.",
  );
} finally {
  await browser.close();
  await new Promise((resolve) => server.close(resolve));
}
