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

async function apiFixture(t, handle) {
  const server = createServer(async (req, res) => {
    try {
      assert.equal(req.headers.authorization, `Bearer ${token}`);
      let raw = "";
      for await (const chunk of req) raw += chunk;
      const body = raw ? JSON.parse(raw) : null;
      const response = await handle(new URL(req.url, "http://localhost"), body);
      const [status, payload] = Array.isArray(response) ? response : [200, response];
      res.writeHead(status, { "Content-Type": "application/json" });
      res.end(JSON.stringify(payload));
    } catch (error) {
      res.writeHead(400, { "Content-Type": "application/json" });
      res.end(JSON.stringify({ error: String(error) }));
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
