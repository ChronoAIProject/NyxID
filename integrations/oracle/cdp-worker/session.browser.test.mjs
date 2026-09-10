import assert from "node:assert/strict";
import { spawn } from "node:child_process";
import { createCipheriv, hkdfSync, randomBytes, randomUUID } from "node:crypto";
import { once } from "node:events";
import { mkdtemp, readFile, rm, writeFile } from "node:fs/promises";
import { createServer } from "node:http";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import test from "node:test";
import { fileURLToPath } from "node:url";
import { chromium } from "playwright-core";
import { decryptSessionEnvelope } from "./worker.mjs";

const chromeExecutable = process.env.NYXID_TEST_CHROME_EXECUTABLE
  || (process.env.NYXID_TEST_BROWSER === "1" ? chromium.executablePath() : undefined);
const workerPath = join(dirname(fileURLToPath(import.meta.url)), "worker.mjs");
const token = "nyx_owk_browser-integration-test-only";
const sessionInfo = Buffer.from("nyxid-oracle-session-v1");

function seal(snapshot) {
  const salt = randomBytes(32);
  const nonce = randomBytes(12);
  const key = Buffer.from(hkdfSync("sha256", token, salt, sessionInfo, 32));
  const cipher = createCipheriv("aes-256-gcm", key, nonce);
  cipher.setAAD(sessionInfo);
  const ciphertext = Buffer.concat([
    cipher.update(JSON.stringify(snapshot)), cipher.final(), cipher.getAuthTag(),
  ]);
  key.fill(0);
  return Buffer.from(JSON.stringify({
    version: 1,
    salt_base64: salt.toString("base64"),
    nonce_base64: nonce.toString("base64"),
    ciphertext_base64: ciphertext.toString("base64"),
  })).toString("base64");
}

async function waitUntil(predicate, timeout = 15000) {
  const deadline = Date.now() + timeout;
  while (Date.now() < deadline) {
    const result = await predicate();
    if (result) return result;
    await new Promise((resolve) => setTimeout(resolve, 50));
  }
  throw new Error("Browser integration condition timed out");
}

async function browserFixture(t, loggedIn = true) {
  const directory = await mkdtemp(join(tmpdir(), "nyxid-session-browser-"));
  const children = [];
  const identity = { status: 200 };
  let context;
  t.after(async () => {
    for (const child of children) {
      if (child.exitCode === null && child.signalCode === null) {
        const exited = once(child, "exit");
        child.kill("SIGKILL");
        await exited;
      }
    }
    await context?.close();
    await rm(directory, { recursive: true, force: true });
  });
  const profile = join(directory, "chrome");
  context = await chromium.launchPersistentContext(profile, {
    executablePath: chromeExecutable,
    headless: true,
    args: ["--remote-debugging-port=0", "--remote-debugging-address=127.0.0.1"],
  });
  await context.route("**/*", async (route) => {
    const url = new URL(route.request().url());
    if (!["chatgpt.com", "auth.openai.com", "unrelated.example"].includes(url.hostname)) {
      return route.abort();
    }
    const session = /(?:^|;\s*)session=([^;]+)/.exec(route.request().headers().cookie || "")?.[1];
    if (url.hostname === "chatgpt.com" && url.pathname === "/api/auth/session") {
      return route.fulfill({
        status: identity.status,
        contentType: "application/json",
        body: JSON.stringify({
          user: session ? { id: `fixture-user-${session}` } : null,
          accessToken: "fixture-access-token-must-stay-in-browser",
        }),
      });
    }
    const imported = ["imported-account", "other-account"].includes(session);
    const authenticated = loggedIn || imported;
    return route.fulfill({
      contentType: "text/html",
      body: authenticated
        ? "<!doctype html><textarea id='prompt-textarea'></textarea>"
        : "<!doctype html><button>Log in</button>",
    });
  });
  const page = context.pages()[0];
  await page.goto("https://chatgpt.com/");
  const port = (await readFile(join(profile, "DevToolsActivePort"), "utf8")).split("\n")[0];
  return { directory, profile, context, page, children, identity, cdp: `http://127.0.0.1:${port}` };
}

function workerProcess(fixture, args, extraEnv = {}) {
  const child = spawn(process.execPath, [workerPath, ...args], {
    env: {
      PATH: process.env.PATH,
      HOME: fixture.directory,
      CHROME_CDP_URL: fixture.cdp,
      CHROME_PROFILE_DIR: fixture.profile,
      NYXID_WORKER_STATE_FILE: join(fixture.directory, "state.json"),
      NYXID_INSTALLATION_ID_FILE: join(fixture.directory, "installation-id"),
      NYXID_WORKER_LABEL: "browser-test",
      NYXID_POLL_MS: "50",
      NYXID_PRESENCE_MS: "50",
      NYXID_HTTP_TIMEOUT_MS: "2000",
      NYXID_LOGIN_CAPTURE_TIMEOUT_MS: "5000",
      NYXID_SAVED_LOGIN_POLL_MS: "100",
      NYXID_SAVED_LOGIN_REFRESH_MS: "200",
      ...extraEnv,
    },
    stdio: ["ignore", "pipe", "pipe"],
  });
  let output = "";
  child.stdout.on("data", (chunk) => { output += chunk; });
  child.stderr.on("data", (chunk) => { output += chunk; });
  fixture.children.push(child);
  return { child, output: () => output };
}

async function apiFixture(t, handle, expectedToken = token) {
  const server = createServer(async (req, res) => {
    try {
      assert.equal(req.headers.authorization, `Bearer ${expectedToken}`);
      let raw = "";
      for await (const chunk of req) raw += chunk;
      const body = raw ? JSON.parse(raw) : null;
      const response = await handle(new URL(req.url, "http://localhost"), body);
      const [status, payload] = Array.isArray(response) ? response : [200, response];
      res.writeHead(status, { "Content-Type": "application/json" });
      res.end(JSON.stringify(payload));
    } catch (error) {
      t.diagnostic(error instanceof Error ? error.message : "Unexpected mock API failure");
      res.writeHead(400, { "Content-Type": "application/json" });
      res.end(JSON.stringify({ error: "Invalid mock API request" }));
    }
  });
  server.listen(0, "127.0.0.1");
  await once(server, "listening");
  t.after(async () => {
    server.closeAllConnections();
    await new Promise((resolve, reject) => server.close((error) => error ? reject(error) : resolve()));
  });
  return `http://127.0.0.1:${server.address().port}`;
}

const options = { skip: !chromeExecutable, timeout: 30000 };

test("an enrolled member worker uses its own account and polls the shared queue without shared-login access", options, async (t) => {
  const fixture = await browserFixture(t, true);
  await fixture.context.addCookies([{
    name: "session", value: "contributor-account", domain: ".chatgpt.com", path: "/", secure: true,
  }]);
  const credential = `nyx_owi_${randomBytes(32).toString("hex")}`;
  const heartbeats = [];
  const routes = [];
  const base = await apiFixture(t, (url, body) => {
    routes.push(url.pathname);
    if (url.pathname.endsWith("/heartbeat")) {
      heartbeats.push(body);
      return { status: "ok" };
    }
    if (url.pathname.endsWith("/task")) return { status: "idle" };
    throw new Error(`Unexpected enrolled worker route ${url.pathname}`);
  }, credential);
  const process = workerProcess(fixture, [], { NYXID_BASE_URL: base, NYXID_WORKER_TOKEN: credential });
  await waitUntil(() => heartbeats.length >= 3 && routes.filter(route => route.endsWith("/task")).length >= 3);
  assert.ok(heartbeats.some(heartbeat => heartbeat.logged_in === true));
  for (const heartbeat of heartbeats) {
    assert.equal(heartbeat.worker, "browser-test");
    assert.match(heartbeat.instance_id, /^[0-9a-f-]{36}$/);
    assert.deepEqual(heartbeat.capabilities, ["commands_v1", "upgrade_v1", "attempt_fencing_v1"]);
  }
  assert.equal(routes.some(route => route.includes("login-profile") || route.includes("login-snapshot")), false);
  assert.equal((await fixture.context.cookies()).find(cookie => cookie.name === "session")?.value, "contributor-account");
  assert.equal(process.output().includes(credential), false);
});

async function savedLoginApi(t, replaceExisting = false) {
  const snapshot = {
    version: 1,
    cookies: [{ name: "session", value: "imported-account", domain: ".chatgpt.com", path: "/", secure: true, httpOnly: true, expires: -1, sameSite: "Lax" }],
    origins: [],
  };
  const state = {
    profile: {
      id: randomUUID(), name: "account-a", generation: randomUUID(), revision: randomUUID(),
      status: "available", envelope_size: 512,
      updated_at: new Date().toISOString(), expires_at: "2099-01-01T00:00:00Z", workers: ["browser-test"],
    },
    binding: { binding_id: randomUUID(), worker_label: "browser-test", replace_existing: replaceExisting },
    envelope: seal(snapshot),
    heartbeats: [], reads: [], publications: [], acknowledgements: [], taskRequests: 0, taskResponse: null,
  };
  const base = await apiFixture(t, (url, body) => {
    if (url.pathname.endsWith("/heartbeat")) {
      state.heartbeats.push(body);
      return { status: "ok" };
    }
    if (url.pathname.endsWith("/task")) {
      state.taskRequests += 1;
      if (typeof state.taskResponse === "function") return state.taskResponse();
      return state.taskResponse || { status: "idle" };
    }
    if (url.pathname.endsWith("/ack")) {
      state.acknowledgements.push(body);
      return { status: "ok" };
    }
    if (url.pathname.endsWith("/login-profile") && !body) {
      state.reads.push(url);
      return {
        status: "available", profile: state.profile, binding: state.binding, format_version: 1,
        sealed_blob_base64: url.searchParams.get("known_revision") === state.profile.revision ? undefined : state.envelope,
      };
    }
    if (url.pathname.endsWith("/login-profile") && body) {
      state.publications.push(body);
      if (body.expected_revision !== state.profile.revision || body.generation !== state.profile.generation) {
        return [409, { error: "saved login changed" }];
      }
      state.profile = { ...state.profile, revision: body.publication_id, updated_at: new Date().toISOString() };
      state.envelope = body.sealed_blob_base64;
      return state.profile;
    }
    throw new Error(`Unexpected worker route ${url.pathname}`);
  });
  return { base, state };
}

test("local CLI capture reads an isolated browser and excludes unrelated credentials", options, async (t) => {
  const fixture = await browserFixture(t);
  await fixture.context.addCookies([
    { name: "session", value: "captured-account", domain: ".chatgpt.com", path: "/", secure: true, httpOnly: true },
    { name: "refresh", value: "path-scoped-session", domain: "auth.openai.com", path: "/api/auth", secure: true, httpOnly: true },
    { name: "foreign", value: "must-not-transfer", domain: "unrelated.example", path: "/", secure: true },
  ]);
  await fixture.page.evaluate(() => localStorage.setItem("session-marker", "capture-storage"));
  const capturePath = join(fixture.directory, "session.json");
  const process = workerProcess(fixture, ["--capture-session", capturePath]);
  const [code] = await once(process.child, "exit");
  assert.equal(code, 0, process.output());
  const captured = JSON.parse(await readFile(capturePath, "utf8"));
  assert.equal(captured.version, 1);
  assert.equal(captured.cookies.find((cookie) => cookie.name === "session")?.value, "captured-account");
  assert.equal(captured.cookies.find((cookie) => cookie.name === "refresh")?.value, "path-scoped-session");
  assert.ok(!captured.cookies.some((cookie) => cookie.name === "foreign"));
  assert.ok(captured.origins.some((origin) => origin.local_storage.some((item) => item.value === "capture-storage")));
  assert.equal(fixture.context.pages().length, 1);
});

test("legacy session-import command runs through the worker HTTP loop and verifies login", options, async (t) => {
  const fixture = await browserFixture(t, false);
  const snapshot = {
    version: 1,
    cookies: [{ name: "session", value: "imported-account", domain: ".chatgpt.com", path: "/", secure: true, httpOnly: true, expires: -1, sameSite: "Lax" }],
    origins: [{ origin: "https://chatgpt.com", local_storage: [{ name: "session-marker", value: "imported-storage" }], session_storage: [] }],
  };
  const envelope = seal(snapshot);
  const heartbeats = [];
  let snapshotRequests = 0;
  const base = await apiFixture(t, (url, body) => {
    if (url.pathname.endsWith("/heartbeat")) {
      heartbeats.push(body);
      return {
        status: "ok",
        command: heartbeats.length === 1
          ? { id: "legacy-import", command: "session_import", snapshot_id: "legacy-snapshot" }
          : undefined,
      };
    }
    if (url.pathname.endsWith("/login-snapshots/legacy-snapshot")) {
      snapshotRequests += 1;
      return { format_version: 1, sealed_blob_base64: envelope };
    }
    if (url.pathname.endsWith("/task")) return { status: "idle" };
    if (url.pathname.endsWith("/login-profile")) return [404, { error: "legacy backend" }];
    throw new Error(`Unexpected worker route ${url.pathname}`);
  });
  const process = workerProcess(fixture, [], { NYXID_BASE_URL: base, NYXID_WORKER_TOKEN: token });
  await waitUntil(() => heartbeats.some((body) => body.command_reports?.some((report) => report.command_id === "legacy-import")));
  const report = heartbeats.flatMap((body) => body.command_reports || []).find((item) => item.command_id === "legacy-import");
  assert.equal(report.succeeded, true, process.output());
  assert.equal(report.result_code, "session_import_verified");
  assert.equal(snapshotRequests, 1);
  assert.equal((await fixture.context.cookies()).find((cookie) => cookie.name === "session")?.value, "imported-account");
  assert.equal(await fixture.page.evaluate(() => localStorage.getItem("session-marker")), "imported-storage");
  assert.equal(fixture.context.pages().length, 1);
  const state = await readFile(join(fixture.directory, "state.json"), "utf8");
  assert.ok(!state.includes("imported-account"));
  assert.ok(!state.includes("imported-storage"));
  assert.ok(!process.output().includes("imported-account"));
});

test("an invalid imported snapshot cannot erase an existing browser login", options, async (t) => {
  const fixture = await browserFixture(t, true);
  await fixture.context.addCookies([
    { name: "session", value: "existing-account-b", domain: ".chatgpt.com", path: "/", secure: true, httpOnly: true },
  ]);
  const reports = [];
  let delivered = false;
  const base = await apiFixture(t, (url, body) => {
    if (url.pathname.endsWith("/heartbeat")) {
      reports.push(...body.command_reports || []);
      const command = delivered ? undefined : { id: "invalid-import", command: "session_import", snapshot_id: "invalid" };
      delivered = true;
      return { status: "ok", command };
    }
    if (url.pathname.endsWith("/login-snapshots/invalid")) {
      return { format_version: 1, sealed_blob_base64: seal({ version: 1, cookies: [], origins: [] }) };
    }
    if (url.pathname.endsWith("/task")) return { status: "idle" };
    if (url.pathname.endsWith("/login-profile")) return [404, { error: "legacy backend" }];
    throw new Error(`Unexpected worker route ${url.pathname}`);
  });
  workerProcess(fixture, [], { NYXID_BASE_URL: base, NYXID_WORKER_TOKEN: token });
  const report = await waitUntil(() => reports.find((item) => item.command_id === "invalid-import"));
  assert.equal(report.succeeded, false);
  assert.equal((await fixture.context.cookies()).find((cookie) => cookie.name === "session")?.value, "existing-account-b");
});

test("a new worker imports a saved login and publishes refreshed cookies encrypted", options, async (t) => {
  const fixture = await browserFixture(t, false);
  const { base, state } = await savedLoginApi(t);
  const process = workerProcess(fixture, [], { NYXID_BASE_URL: base, NYXID_WORKER_TOKEN: token });
  await waitUntil(() => state.heartbeats.some((body) => body.logged_in === true));
  assert.equal((await fixture.context.cookies()).find((cookie) => cookie.name === "session")?.value, "imported-account");
  await fixture.context.addCookies([
    { name: "refresh", value: "new-browser-refresh", domain: "auth.openai.com", path: "/api/auth", secure: true, httpOnly: true },
  ]);
  const published = await waitUntil(() => state.publications.find((body) => {
    const capture = decryptSessionEnvelope(Buffer.from(body.sealed_blob_base64, "base64"), token);
    return capture.cookies.some((cookie) => cookie.name === "refresh" && cookie.value === "new-browser-refresh");
  }));
  assert.equal(published.profile_id, state.profile.id);
  assert.equal(published.binding_id, state.binding.binding_id);
  assert.equal(published.generation, state.profile.generation);
  assert.ok(!JSON.stringify(published).includes("new-browser-refresh"));
  assert.ok(!process.output().includes("new-browser-refresh"));
  const localState = await readFile(join(fixture.directory, "state.json"), "utf8");
  assert.ok(!localState.includes("new-browser-refresh"));
  assert.ok(!localState.includes("imported-account"));
  assert.ok(!localState.includes("fixture-user-"));
  assert.ok(!process.output().includes("fixture-access-token"));
  assert.equal(fixture.context.pages().length, 1);
});

test("binding a saved login preserves an already authenticated account by default", options, async (t) => {
  const fixture = await browserFixture(t, true);
  await fixture.context.addCookies([
    { name: "session", value: "existing-account-b", domain: ".chatgpt.com", path: "/", secure: true, httpOnly: true },
  ]);
  const { base, state } = await savedLoginApi(t);
  workerProcess(fixture, [], { NYXID_BASE_URL: base, NYXID_WORKER_TOKEN: token });
  await waitUntil(() => state.reads.length >= 3);
  assert.equal((await fixture.context.cookies()).find((cookie) => cookie.name === "session")?.value, "existing-account-b");
  assert.equal(state.publications.length, 0, "an unverified binding must not publish the existing account into the saved profile");
});

test("a browser account switch cannot publish the new account into the previous saved profile", options, async (t) => {
  const fixture = await browserFixture(t, false);
  const { base, state } = await savedLoginApi(t);
  workerProcess(fixture, [], { NYXID_BASE_URL: base, NYXID_WORKER_TOKEN: token });
  await waitUntil(() => state.publications.length > 0);
  await fixture.page.goto("https://auth.openai.com/log-in");
  await fixture.context.addCookies([
    { name: "session", value: "other-account", domain: ".chatgpt.com", path: "/", secure: true, httpOnly: true },
  ]);
  await fixture.page.goto("https://chatgpt.com/");
  const readCount = state.reads.length;
  await waitUntil(() => state.reads.length >= readCount + 6);
  const local = JSON.parse(await readFile(join(fixture.directory, "state.json"), "utf8"));
  assert.equal(local.saved_login.status, "external_login");
  assert.equal((await fixture.context.cookies()).find((cookie) => cookie.name === "session")?.value, "other-account");
  for (const publication of state.publications) {
    const snapshot = decryptSessionEnvelope(Buffer.from(publication.sealed_blob_base64, "base64"), token);
    assert.ok(!snapshot.cookies.some((cookie) => cookie.value === "other-account"));
  }
});

for (const staleSibling of [false, true]) {
  test(`an account changed while the worker was stopped is preserved with ${staleSibling ? "a stale sibling" : "its own"} revision`, options, async (t) => {
    const fixture = await browserFixture(t, false);
    const { base, state } = await savedLoginApi(t);
    const first = workerProcess(fixture, [], { NYXID_BASE_URL: base, NYXID_WORKER_TOKEN: token });
    await waitUntil(() => state.publications.length > 0);
    await waitUntil(async () => {
      const local = JSON.parse(await readFile(join(fixture.directory, "state.json"), "utf8"));
      return local.saved_login.source_revision === state.profile.revision;
    });
    const exited = once(first.child, "exit");
    first.child.kill("SIGKILL");
    await exited;
    const publicationCount = state.publications.length;
    await fixture.context.addCookies([
      { name: "session", value: "other-account", domain: ".chatgpt.com", path: "/", secure: true, httpOnly: true },
    ]);
    await fixture.page.reload();
    if (staleSibling) {
      state.profile = { ...state.profile, revision: randomUUID(), updated_at: new Date(Date.now() - 60000).toISOString() };
    }
    const reads = state.reads.length;
    workerProcess(fixture, [], { NYXID_BASE_URL: base, NYXID_WORKER_TOKEN: token });
    await waitUntil(() => state.reads.length >= reads + 6);
    assert.equal(state.publications.length, publicationCount);
    assert.equal((await fixture.context.cookies()).find((cookie) => cookie.name === "session")?.value, "other-account");
    const local = JSON.parse(await readFile(join(fixture.directory, "state.json"), "utf8"));
    assert.equal(local.saved_login.status, "external_login");
  });
}

test("a temporary account identity outage pauses publication while task polling continues", options, async (t) => {
  const fixture = await browserFixture(t, false);
  const { base, state } = await savedLoginApi(t);
  workerProcess(fixture, [], { NYXID_BASE_URL: base, NYXID_WORKER_TOKEN: token });
  await waitUntil(() => state.publications.length > 0);
  fixture.identity.status = 503;
  await waitUntil(() => state.heartbeats.some((body) => body.last_error === "saved_login_identity_unavailable"));
  const failedHeartbeat = state.heartbeats.findIndex((body) => body.last_error === "saved_login_identity_unavailable");
  const publications = state.publications.length;
  const tasks = state.taskRequests;
  const reads = state.reads.length;
  await waitUntil(() => state.reads.length >= reads + 5);
  assert.equal(state.publications.length, publications);
  assert.ok(state.taskRequests > tasks);
  assert.ok(state.heartbeats.slice(failedHeartbeat).every((body) => body.last_error === "saved_login_identity_unavailable"));
  fixture.identity.status = 200;
  await waitUntil(() => state.publications.length > publications);
  const local = JSON.parse(await readFile(join(fixture.directory, "state.json"), "utf8"));
  assert.equal(local.saved_login.status, "verified");
  assert.match(local.saved_login.account_fingerprint, /^[a-f0-9]{64}$/);
});

test("an import without an account identity cannot later trust an arbitrary browser account", options, async (t) => {
  const fixture = await browserFixture(t, false);
  fixture.identity.status = 503;
  const { base, state } = await savedLoginApi(t);
  workerProcess(fixture, [], { NYXID_BASE_URL: base, NYXID_WORKER_TOKEN: token });
  await waitUntil(() => state.heartbeats.some((body) => body.logged_in === true && body.last_error === "saved_login_identity_unavailable"));
  fixture.identity.status = 200;
  await fixture.context.addCookies([
    { name: "session", value: "other-account", domain: ".chatgpt.com", path: "/", secure: true, httpOnly: true },
  ]);
  const reads = state.reads.length;
  await waitUntil(() => state.reads.length >= reads + 5);
  assert.equal(state.publications.length, 0);
  assert.equal((await fixture.context.cookies()).find((cookie) => cookie.name === "session")?.value, "other-account");
  state.profile = { ...state.profile, generation: randomUUID(), revision: randomUUID(), updated_at: new Date().toISOString() };
  await waitUntil(() => state.publications.length > 0);
  assert.equal((await fixture.context.cookies()).find((cookie) => cookie.name === "session")?.value, "imported-account");
});

test("a sibling refresh does not authorize publishing cookies from an older source revision", options, async (t) => {
  const fixture = await browserFixture(t, false);
  const { base, state } = await savedLoginApi(t);
  workerProcess(fixture, [], { NYXID_BASE_URL: base, NYXID_WORKER_TOKEN: token, NYXID_SAVED_LOGIN_REFRESH_MS: "2000" });
  await waitUntil(() => state.publications.length > 0);
  const siblingRevision = randomUUID();
  state.profile = { ...state.profile, revision: siblingRevision, updated_at: new Date().toISOString() };
  const publicationCount = state.publications.length;
  const readCount = state.reads.length;
  await fixture.context.addCookies([
    { name: "refresh", value: "stale-worker-refresh", domain: "auth.openai.com", path: "/api/auth", secure: true, httpOnly: true },
  ]);
  await waitUntil(() => state.reads.length >= readCount + 5);
  assert.ok(state.publications.slice(publicationCount).every((body) => body.expected_revision !== siblingRevision));
  assert.equal(state.profile.revision, siblingRevision);
  assert.equal((await fixture.context.cookies()).find((cookie) => cookie.name === "session")?.value, "imported-account");
});

test("an idle worker adopts the current source before taking over stale refresh publication", options, async (t) => {
  const fixture = await browserFixture(t, false);
  const { base, state } = await savedLoginApi(t);
  workerProcess(fixture, [], { NYXID_BASE_URL: base, NYXID_WORKER_TOKEN: token });
  await waitUntil(() => state.publications.length > 0);
  const source = decryptSessionEnvelope(Buffer.from(state.envelope, "base64"), token);
  source.cookies.push({ name: "new-source", value: "latest-sibling-cookie", domain: ".chatgpt.com", path: "/", secure: true, httpOnly: true, expires: -1, sameSite: "Lax" });
  const siblingRevision = randomUUID();
  state.envelope = seal(source);
  state.profile = { ...state.profile, revision: siblingRevision, updated_at: new Date(Date.now() - 60000).toISOString() };
  const takeover = await waitUntil(() => state.publications.find((body) => body.expected_revision === siblingRevision));
  const capture = decryptSessionEnvelope(Buffer.from(takeover.sealed_blob_base64, "base64"), token);
  assert.equal(capture.cookies.find((cookie) => cookie.name === "new-source")?.value, "latest-sibling-cookie");
});

test("a rejected automatic handover restores the healthy browser and reports its failure", { ...options, timeout: 90000 }, async (t) => {
  const fixture = await browserFixture(t, false);
  const { base, state } = await savedLoginApi(t);
  workerProcess(fixture, [], { NYXID_BASE_URL: base, NYXID_WORKER_TOKEN: token });
  await waitUntil(() => state.publications.length > 0);
  const snapshot = decryptSessionEnvelope(Buffer.from(state.envelope, "base64"), token);
  snapshot.cookies.find((cookie) => cookie.name === "session").value = "rejected-session";
  const rejectedRevision = randomUUID();
  state.envelope = seal(snapshot);
  state.profile = { ...state.profile, revision: rejectedRevision, updated_at: new Date(Date.now() - 60000).toISOString() };
  await waitUntil(async () => (await fixture.context.cookies()).some((cookie) => cookie.value === "rejected-session"));
  await waitUntil(async () => {
    const local = JSON.parse(await readFile(join(fixture.directory, "state.json"), "utf8"));
    return local.saved_login?.status === "failed";
  }, 75000);
  assert.equal((await fixture.context.cookies()).find((cookie) => cookie.name === "session")?.value, "imported-account");
  await waitUntil(() => state.heartbeats.some((body) => body.logged_in === true && body.last_error?.includes("session_import_verification_failed")));
  const readCount = state.reads.length;
  await waitUntil(() => state.reads.length >= readCount + 5);
  assert.equal((await fixture.context.cookies()).find((cookie) => cookie.name === "session")?.value, "imported-account");
  assert.ok(!state.publications.some((body) => body.expected_revision === rejectedRevision));
});

test("a persisted possible send prevents saved-login import from navigating the conversation", options, async (t) => {
  const fixture = await browserFixture(t, false);
  await fixture.page.goto("https://chatgpt.com/c/active-conversation");
  await fixture.context.addCookies([
    { name: "session", value: "task-account", domain: ".chatgpt.com", path: "/", secure: true, httpOnly: true },
  ]);
  await writeFile(join(fixture.directory, "state.json"), JSON.stringify({
    format_version: 1, instance_id: randomUUID(),
    current_task: { task_id: "active-task", dispatch_attempt_id: "active-attempt", phase: "send_attempted", conversation_url: fixture.page.url() },
  }), { mode: 0o600 });
  const { base, state } = await savedLoginApi(t, true);
  let releasePoll;
  state.taskResponse = new Promise((resolve) => { releasePoll = resolve; });
  try {
    workerProcess(fixture, [], { NYXID_BASE_URL: base, NYXID_WORKER_TOKEN: token });
    await waitUntil(() => state.taskRequests > 0);
    assert.ok(state.reads.length > 0);
    assert.equal(fixture.page.url(), "https://chatgpt.com/c/active-conversation");
    assert.equal((await fixture.context.cookies()).find((cookie) => cookie.name === "session")?.value, "task-account");
    assert.equal(state.publications.length, 0);
  } finally {
    releasePoll({ status: "idle" });
  }
});

test("a fresh saved login recovers a task that logs out before sending", options, async (t) => {
  const fixture = await browserFixture(t, false);
  await fixture.context.route("https://chatgpt.com/**", async (route) => {
    if (new URL(route.request().url()).pathname === "/api/auth/session") return route.fallback();
    const authenticated = route.request().headers().cookie?.includes("session=imported-account");
    await route.fulfill({
      contentType: "text/html",
      body: authenticated
        ? "<!doctype html><textarea id='prompt-textarea'></textarea><button data-testid='send-button'>Send</button>"
        : "<!doctype html><button>Log in</button>",
    });
  });
  const { base, state } = await savedLoginApi(t);
  workerProcess(fixture, [], { NYXID_BASE_URL: base, NYXID_WORKER_TOKEN: token });
  await waitUntil(() => state.publications.length > 0);
  let dispatched = false;
  const generation = randomUUID();
  state.taskResponse = async () => {
    if (dispatched) return { status: "idle" };
    dispatched = true;
    await fixture.context.clearCookies();
    await fixture.page.reload();
    state.profile = { ...state.profile, generation, revision: randomUUID(), updated_at: new Date().toISOString() };
    return {
      status: "task", kind: "prompt", task_id: "pre-send-login", dispatch_attempt_id: "attempt-one",
      prompt: "Recover this prompt once", model: "unknown", is_followup: false,
    };
  };
  await waitUntil(() => state.acknowledgements.some((body) => body.phase === "sent"), 20000);
  const local = JSON.parse(await readFile(join(fixture.directory, "state.json"), "utf8"));
  assert.equal(local.saved_login.generation, generation);
  assert.equal(local.saved_login.status, "verified");
  assert.equal(local.current_task.recovery_failures || 0, 0);
  assert.equal(state.acknowledgements.filter((body) => body.phase === "sent").length, 1);
});

test("an older backend does not turn a logged-out worker into a saved-login failure", options, async (t) => {
  const fixture = await browserFixture(t, false);
  const heartbeats = [];
  let lookupCount = 0;
  const base = await apiFixture(t, (url, body) => {
    if (url.pathname.endsWith("/heartbeat")) {
      heartbeats.push(body);
      return { status: "ok" };
    }
    if (url.pathname.endsWith("/login-profile")) {
      lookupCount += 1;
      return [404, { error: "legacy backend" }];
    }
    if (url.pathname.endsWith("/task")) return { status: "idle" };
    throw new Error(`Unexpected worker route ${url.pathname}`);
  });
  workerProcess(fixture, [], { NYXID_BASE_URL: base, NYXID_WORKER_TOKEN: token });
  await waitUntil(() => lookupCount > 0 && heartbeats.length >= 3);
  assert.equal(heartbeats.at(-1).last_error, null);
  assert.equal(fixture.context.pages().length, 1);
});

// Real worker process + real pointer clicks, but no upstream ChatGPT account.
// Menus reproduce Radix's body pointer-events lock, including sticky submenus.
function reasoningPage(config) {
  const region = config.noForm ? "section" : "form";
  return `<!doctype html><meta charset="utf-8"><style>
    body { margin: 0; } main { padding: 40px; min-height: 500px; }
    form, section { margin-top: ${config.offscreen ? 1500 : 100}px; width: 600px; }
    #prompt-textarea { display: block; width: 500px; height: 100px; border: 1px solid; }
    [role=menu], [role=listbox] { position: fixed; top: 80px; left: 60px;
      background: white; border: 1px solid; padding: 10px; pointer-events: auto; z-index: 10; }
    [role=menuitemradio], [role=menuitem] { display: block; padding: 6px; }
    #sidebar { position: fixed; top: 0; left: 680px; }
  </style><header><button id="header-model" aria-haspopup="menu">GPT-6</button></header>
  ${config.sidebar ? '<nav id="sidebar" role="listbox"><button role="option" id="sidebar-pro">Pro</button></nav>' : ''}
  <main><div id="turns"></div><${region}>
  ${config.contenteditable ? '<div id="prompt-textarea" contenteditable="true"></div>' : '<textarea id="prompt-textarea"></textarea>'}
  ${config.twoPills ? '<button type="button" id="tools-pill" class="__composer-pill" aria-haspopup="menu">GPT tools</button>' : ''}
  ${config.noPill ? "" : `<button type="button" id="pill" class="${config.fallback ? "" : "__composer-pill"}" aria-haspopup="menu">${config.initial || "自动"}</button>`}
  <button type="button" data-testid="send-button">Send</button>
  </${region}></main><script>
    const config = ${JSON.stringify(config)};
    window.clickLog = [];
    let typed = false;
    const record = (event) => window.clickLog.push({event, typed});
    const input = document.querySelector('#prompt-textarea');
    const pill = document.querySelector('#pill');
    const closeMenu = () => {
      document.querySelectorAll('[data-picker-menu]').forEach(el => el.remove());
      document.body.style.pointerEvents = '';
    };
    const renderMenu = (nested = false) => {
      closeMenu();
      const menu = document.createElement('div');
      menu.setAttribute('data-picker-menu', '');
      menu.setAttribute('role', config.listbox ? 'listbox' : 'menu');
      document.body.style.pointerEvents = 'none';
      const labels = nested && config.arbitraryNested
        ? ['Instant', 'Profile', 'Delete conversation'] : ['Instant', 'Medium', 'High', 'Extra High', 'Pro'];
      for (const label of labels.filter(label => !config.missingLevel || label !== 'Pro')) {
        const item = document.createElement('button');
        item.textContent = label;
        if (config.hiddenHints) {
          const hint = document.createElement('span');
          hint.style.display = 'none';
          hint.textContent = 'private hidden hint for ' + label;
          item.append(hint);
        }
        item.setAttribute('role', 'menuitemradio');
        item.setAttribute('aria-checked', String(nested && (config.arbitraryNested ? label === 'Profile' : label === 'Instant')));
        item.onclick = () => {
          record('level:' + label);
          if (!config.unverified) pill.textContent = 'GPT-6 ' + label;
          if (!nested && (config.sticky || config.arbitraryNested)) renderMenu(true);
          else if (!config.sticky) closeMenu();
        };
        menu.append(item);
      }
      document.body.append(menu);
    };
    document.querySelector('#header-model').onclick = () => { record('header'); renderMenu(); };
    if (config.blockPicker || config.slowPickerClick) {
      const rect = pill.getBoundingClientRect();
      const blocker = document.createElement('div');
      blocker.id = 'picker-blocker';
      blocker.style.cssText = 'position:fixed;z-index:20;background:white;left:' + rect.x + 'px;top:' + rect.y +
        'px;width:' + rect.width + 'px;height:' + rect.height + 'px';
      document.body.append(blocker);
    }
    document.querySelector('#sidebar-pro')?.addEventListener('click', () => record('sidebar:Pro'));
    document.querySelector('#tools-pill')?.addEventListener('click', () => record('tools-picker'));
    if (pill) pill.onclick = () => {
      record('picker');
      if (config.neverOpens) return;
      renderMenu();
    };
    document.addEventListener('keydown', event => {
      if (event.key === 'Escape') {
        record('escape');
        if (!config.permanentBlock) closeMenu();
      }
    });
    input.addEventListener('input', () => {
      typed = true;
      const rect = input.getBoundingClientRect();
      window.composerVisibleAtFill = rect.top >= 0 && rect.bottom <= innerHeight;
      record('typed');
      if (config.blockPicker) setTimeout(() => document.querySelector('#picker-blocker').remove(), 600);
      if (config.obstructAfterFill) renderMenu();
      if (config.permanentBlock) document.body.style.pointerEvents = 'none';
    });
    document.querySelector('[data-testid=send-button]').onclick = () => {
      record('send');
      const user = document.createElement('div');
      user.setAttribute('data-message-author-role', 'user');
      user.textContent = input.value || input.textContent;
      const assistant = document.createElement('div');
      assistant.setAttribute('data-message-author-role', 'assistant');
      assistant.innerHTML = '<div class="markdown">Synthetic reasoning response</div>';
      document.querySelector('#turns').append(user, assistant);
      history.pushState({}, '', '/c/aaaaaa-bbbbbb');
    };
  </script>`;
}

async function reasoningFixture(t, config = {}, cancelPhase) {
  const fixture = await browserFixture(t);
  await fixture.context.route('https://chatgpt.com/', route => route.fulfill({ contentType: 'text/html', body: reasoningPage(config) }));
  await fixture.page.goto('https://chatgpt.com/');
  const acknowledgements = [];
  const results = [];
  let claimed = false;
  const task = { status: 'task', kind: 'prompt', task_id: 'reasoning-task', dispatch_attempt_id: 'reasoning-attempt',
    prompt: 'Synthetic private prompt marker', model: 'chatgpt-6-pro', is_followup: false };
  const base = await apiFixture(t, async (url, body) => {
    if (url.pathname.endsWith('/heartbeat')) return { status: 'ok' };
    if (url.pathname.endsWith('/login-profile')) return [404, { error: 'legacy backend' }];
    if (url.pathname.endsWith('/task')) {
      if (claimed) return { status: 'idle' };
      claimed = true;
      return task;
    }
    if (url.pathname.endsWith('/ack')) {
      acknowledgements.push(body);
      if (body.phase === 'selecting_model') {
        if (!body.phase_detail && config.slowPickerClick) {
          await fixture.page.evaluate(() => setTimeout(() => document.querySelector('#picker-blocker').remove(), 1400));
        }
        if (!body.phase_detail && config.expiredRead) {
          await fixture.page.evaluate(() => { const now = Date.now; Date.now = () => now() + 60000; });
        }
        if (body.phase_detail && config.slowComposer) {
          await fixture.page.evaluate(() => {
            const input = document.querySelector('#prompt-textarea');
            input.style.display = 'none';
            setTimeout(() => { input.style.display = ''; }, 6000);
          });
        }
      }
      const cancelled = cancelPhase === 'selection_finished'
        ? body.phase === 'selecting_model' && !!body.phase_detail : body.phase === cancelPhase;
      return { status: cancelled ? 'cancelled' : 'ok' };
    }
    if (url.pathname.endsWith('/pin-conv-url')) return { status: 'ok' };
    if (url.pathname.endsWith('/result')) { results.push(body); return { status: 'completed' }; }
    throw new Error(`Unexpected worker route ${url.pathname}`);
  });
  const process = workerProcess(fixture, [], {
    NYXID_BASE_URL: base, NYXID_WORKER_TOKEN: token,
    NYXID_MODEL_SELECT_TIMEOUT_MS: config.selectionTimeout || (config.blockPicker ? '350' : config.neverOpens ? '1500' : '5000'),
    NYXID_MAX_TASK_RECOVERY_FAILURES: '1',
    // Exercise the existing final extraction after one stability poll so a
    // whole worker/result cycle fits the per-test 30s CI limit.
    NYXID_MAX_WAIT_MS: '1',
  });
  return { ...fixture, process, acknowledgements, results, task };
}

async function assertReasoningDelivered(fixture, { model = 'GPT-6 Pro', detail = 'selected=Pro' } = {}) {
  await waitUntil(() => fixture.results.length > 0, 22000);
  assert.equal(fixture.results.length, 1, fixture.process.output());
  assert.equal(fixture.results[0].response, 'Synthetic reasoning response', fixture.process.output());
  assert.equal(fixture.results[0].model, model);
  assert.equal(fixture.results[0].error, undefined);
  const acks = fixture.acknowledgements;
  assert.deepEqual(acks.map(body => body.phase), ['page_ready', 'selecting_model', 'selecting_model', 'ready_to_send', 'sent']);
  assert.equal(acks[2].phase_detail, detail, fixture.process.output());
  assert.equal(acks.filter(body => body.phase === 'sent').length, 1);
  const events = await fixture.page.evaluate(() => window.clickLog);
  assert.equal(events.filter(item => item.event === 'send').length, 1);
  assert.equal(events.filter(item => item.event === 'header').length, 0);
  assert.deepEqual(await fixture.page.evaluate(() => ({
    menus: document.querySelectorAll('[data-picker-menu]').length,
    blocked: getComputedStyle(document.body).pointerEvents === 'none',
  })), { menus: 0, blocked: false });
  for (const value of [fixture.task.prompt, 'Synthetic reasoning response', '/c/aaaaaa-bbbbbb']) {
    assert.ok(!fixture.process.output().includes(value), fixture.process.output());
    if (!value.startsWith('/c/')) assert.ok(!JSON.stringify(acks).includes(value));
    assert.ok(!JSON.stringify(acks.map(body => body.phase_detail)).includes(value));
  }
  assert.ok(acks.every(body => body.page_url?.startsWith('https://chatgpt.com/')));
  assert.equal(acks.at(-1).page_url, 'https://chatgpt.com/c/aaaaaa-bbbbbb');
  return events;
}

test('reasoning: unrecognized structural pill selects Pro and reports the observed pill', options, async (t) => {
  const fixture = await reasoningFixture(t, { contenteditable: true });
  const events = await assertReasoningDelivered(fixture);
  assert.equal(events.filter(item => item.event === 'picker').length, 1);
  assert.deepEqual(events.filter(item => item.event.startsWith('level:')).map(item => item.event), ['level:Pro']);
});

test('reasoning: sticky submenu commits the target over checked Instant and closes before typing', options, async (t) => {
  const fixture = await reasoningFixture(t, { sticky: true, listbox: true });
  const events = await assertReasoningDelivered(fixture);
  assert.deepEqual(events.filter(item => item.event.startsWith('level:')).map(item => item.event), ['level:Pro', 'level:Pro']);
  assert.ok(events.some(item => item.event === 'escape' && !item.typed));
});

test('reasoning: missing menu times out without a selection interaction after typing', options, async (t) => {
  const fixture = await reasoningFixture(t, { neverOpens: true });
  const events = await assertReasoningDelivered(fixture, { model: '自动', detail: 'timeout' });
  assert.deepEqual(events.map(item => item.event), ['picker', 'typed', 'send']);
  assert.match(fixture.process.output(), /model_selection reason=timeout .*pill_source=structural pill_level=unrecognized pill_text_length=2 items=0 recognized=\[\]/);
});

for (const phase of ['selecting_model', 'selection_finished', 'ready_to_send']) {
  test(`reasoning: cancelled ${phase} acknowledgement prevents Send`, options, async (t) => {
    const fixture = await reasoningFixture(t, {}, phase);
    await waitUntil(async () => {
      if (!fixture.acknowledgements.some(body => body.phase === (phase === 'selection_finished' ? 'selecting_model' : phase) &&
          (phase !== 'selection_finished' || body.phase_detail))) return false;
      return !JSON.parse(await readFile(join(fixture.directory, 'state.json'), 'utf8')).current_task;
    }, 12000);
    assert.equal(fixture.results.length, 0, fixture.process.output());
    assert.equal(fixture.acknowledgements.some(body => body.phase === 'sent'), false);
    const events = await fixture.page.evaluate(() => window.clickLog);
    assert.equal(events.some(item => item.event === 'send'), false);
    if (phase === 'selecting_model') assert.deepEqual(events, []);
    if (phase === 'ready_to_send') assert.ok(events.some(item => item.event === 'typed'));
  });
}

test('reasoning: a matching header picker outside the composer is never clicked', options, async (t) => {
  const fixture = await reasoningFixture(t, { noPill: true });
  const events = await assertReasoningDelivered(fixture, { model: 'chatgpt-6-pro', detail: 'picker_unavailable' });
  assert.deepEqual(events.map(item => item.event), ['typed', 'send']);
  assert.match(fixture.process.output(), /reason=picker_unavailable .*pill_source=none pill_level=unrecognized pill_text_length=0 items=0 recognized=\[\]/);
});

for (const noForm of [false, true]) {
  test(`reasoning: composer-local fallback discovers an unrecognized pill (${noForm ? 'send ancestor' : 'nearest form'})`, options, async (t) => {
    const fixture = await reasoningFixture(t, { fallback: true, noForm });
    await assertReasoningDelivered(fixture);
  });
}

test('reasoning: arbitrary nested entries are dismissed without a blind first or checked click', options, async (t) => {
  const fixture = await reasoningFixture(t, { arbitraryNested: true });
  const events = await assertReasoningDelivered(fixture);
  assert.deepEqual(events.filter(item => item.event.startsWith('level:')).map(item => item.event), ['level:Pro']);
  assert.ok(!fixture.process.output().includes('Delete conversation'));
});

test('reasoning: an unverified click reports the unchanged observed pill', options, async (t) => {
  const fixture = await reasoningFixture(t, { initial: 'Instant', unverified: true });
  await assertReasoningDelivered(fixture, { model: 'Instant', detail: 'unverified=Pro' });
  assert.match(fixture.process.output(), /reason=unverified .*pill_source=structural pill_level=Instant pill_text_length=7 items=5 recognized=\[Instant,Medium,High,Extra High,Pro\]/);
});

test('reasoning: the second composer guard clears a menu opened during fill', options, async (t) => {
  const fixture = await reasoningFixture(t, { obstructAfterFill: true });
  const events = await assertReasoningDelivered(fixture);
  assert.ok(events.some(item => item.event === 'escape' && item.typed));
});

test('reasoning: a permanent obstruction enters pre-send recovery and exhausts safely', options, async (t) => {
  const fixture = await reasoningFixture(t, { permanentBlock: true });
  await waitUntil(() => fixture.results.length > 0, 15000);
  assert.equal(fixture.results[0].response, 'ERROR: browser_recovery_exhausted', fixture.process.output());
  assert.equal(fixture.acknowledgements.some(body => body.phase === 'sent'), false);
  assert.equal(fixture.acknowledgements.filter(body => body.phase === 'page_ready').length, 1);
  assert.equal((await fixture.page.evaluate(() => window.clickLog)).some(item => item.event === 'send'), false);
  assert.match(fixture.process.output(), /composer_unobstructed_failed/);
  assert.ok(fixture.process.output().includes('paused for browser recovery (composer_unobstructed_failed)'));
});


test('reasoning: a timed-out actionability wait is aborted before the covered pill becomes clickable', options, async (t) => {
  const fixture = await reasoningFixture(t, { blockPicker: true });
  const events = await assertReasoningDelivered(fixture, { model: '自动', detail: 'timeout' });
  assert.deepEqual(events.map(item => item.event), ['typed', 'send']);
  assert.equal(await fixture.page.locator('#picker-blocker').count(), 0);
});


test('reasoning: an always-visible sidebar listbox is not the picker or a composer obstruction', options, async (t) => {
  const fixture = await reasoningFixture(t, { sidebar: true });
  const events = await assertReasoningDelivered(fixture);
  assert.equal(events.some(item => item.event === 'sidebar:Pro'), false);
  assert.equal(events.filter(item => item.event === 'level:Pro').length, 1);
  assert.ok(events.filter(item => item.event === 'escape' && !item.typed).length <= 3);
  assert.equal(await fixture.page.locator('#sidebar').isVisible(), true);
});

test('reasoning: hidden item hints do not prevent exact visible-label selection', options, async (t) => {
  const fixture = await reasoningFixture(t, { hiddenHints: true });
  const events = await assertReasoningDelivered(fixture);
  assert.deepEqual(events.filter(item => item.event.startsWith('level:')).map(item => item.event), ['level:Pro']);
  assert.ok(!fixture.process.output().includes('private hidden hint'));
});

test('reasoning: a recognized model pill outranks an earlier structural tools pill', options, async (t) => {
  const fixture = await reasoningFixture(t, { twoPills: true, initial: 'Instant' });
  const events = await assertReasoningDelivered(fixture);
  assert.equal(events.some(item => item.event === 'tools-picker'), false);
  assert.equal(events.filter(item => item.event === 'picker').length, 1);
});

test('reasoning: a missing menu with budget remaining reports menu_not_opened', options, async (t) => {
  const fixture = await reasoningFixture(t, { neverOpens: true, selectionTimeout: '15000' });
  const events = await assertReasoningDelivered(fixture, { model: '自动', detail: 'menu_not_opened' });
  assert.deepEqual(events.map(item => item.event), ['picker', 'typed', 'send']);
  assert.match(fixture.process.output(), /reason=menu_not_opened .*pill_source=structural pill_level=unrecognized pill_text_length=2 items=0 recognized=\[\]/);
});

test('reasoning: unavailable levels log counts and canonical levels without raw labels', options, async (t) => {
  const fixture = await reasoningFixture(t, { missingLevel: true });
  await assertReasoningDelivered(fixture, { model: '自动', detail: 'level_unavailable' });
  assert.match(fixture.process.output(), /reason=level_unavailable .*pill_source=structural pill_level=unrecognized pill_text_length=2 items=4 recognized=\[Instant,Medium,High,Extra High\]/);
  assert.ok(!fixture.process.output().includes('自动'));
});

test('reasoning: expired page-side reads carry a deadline code instead of a TypeError', options, async (t) => {
  const fixture = await reasoningFixture(t, { expiredRead: true });
  await waitUntil(() => fixture.results.length > 0, 12000);
  assert.equal(fixture.acknowledgements.find(body => body.phase_detail)?.phase_detail, 'interaction_deadline');
  assert.match(fixture.process.output(), /model_selection reason=interaction_deadline/);
  assert.ok(!fixture.process.output().includes('TypeError'));
  assert.ok(!fixture.process.output().includes('reason=selection_failed'));
  assert.equal(fixture.results[0].response, 'ERROR: browser_recovery_exhausted');
});

test('reasoning: initial composer visibility may take longer than an action timeout', options, async (t) => {
  const fixture = await reasoningFixture(t, { slowComposer: true });
  await assertReasoningDelivered(fixture);
  assert.ok(!fixture.process.output().includes('paused for browser recovery'));
});

test('reasoning: picker actionability can take longer than one second', options, async (t) => {
  const fixture = await reasoningFixture(t, { slowPickerClick: true });
  await assertReasoningDelivered(fixture);
  assert.equal(await fixture.page.locator('#picker-blocker').count(), 0);
});

test('reasoning: the guard scrolls an off-viewport composer into view', options, async (t) => {
  const fixture = await reasoningFixture(t, { offscreen: true, initial: 'GPT-6 Pro' });
  await assertReasoningDelivered(fixture);
  assert.equal(await fixture.page.evaluate(() => window.composerVisibleAtFill), true);
});
