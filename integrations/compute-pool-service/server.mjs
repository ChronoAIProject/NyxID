#!/usr/bin/env node
import { createHash, randomUUID, timingSafeEqual } from "node:crypto";
import { createServer } from "node:http";
import { readFile, rename, writeFile } from "node:fs/promises";
import { dirname, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const __dirname = dirname(fileURLToPath(import.meta.url));

const HOST = process.env.HOST ?? "127.0.0.1";
const PORT = Number.parseInt(process.env.PORT ?? "8787", 10);
const STORE_PATH = resolve(
  process.env.COMPUTE_POOL_STORE ?? `${__dirname}/.compute-pool-store.json`,
);
const API_TOKEN = process.env.COMPUTE_POOL_API_TOKEN ?? "";
const WORKER_TOKEN = process.env.COMPUTE_POOL_WORKER_TOKEN ?? "";
const DEV_INSECURE = process.env.COMPUTE_POOL_DEV_INSECURE === "1";
const TASK_TIMEOUT_SECS = Number.parseInt(
  process.env.COMPUTE_POOL_TASK_TIMEOUT_SECS ?? "7200",
  10,
);
const RETENTION_DAYS = Number.parseInt(
  process.env.COMPUTE_POOL_RETENTION_DAYS ?? "30",
  10,
);
const MAX_BODY_BYTES = Number.parseInt(
  process.env.COMPUTE_POOL_MAX_BODY_BYTES ?? String(8 * 1024 * 1024),
  10,
);

if (!DEV_INSECURE && (!API_TOKEN || !WORKER_TOKEN)) {
  console.error(
    "COMPUTE_POOL_API_TOKEN and COMPUTE_POOL_WORKER_TOKEN are required. " +
      "Set COMPUTE_POOL_DEV_INSECURE=1 only for local throwaway testing.",
  );
  process.exit(1);
}

let store = await loadStore();

const server = createServer(async (req, res) => {
  try {
    await route(req, res);
  } catch (err) {
    if (err?.status) {
      sendJson(res, err.status, { error: err.message });
      return;
    }
    console.error("request failed:", err?.message ?? String(err));
    sendJson(res, 500, { error: "internal_error" });
  }
});

server.listen(PORT, HOST, () => {
  console.error(`compute-pool-service listening on http://${HOST}:${PORT}`);
});

async function route(req, res) {
  const url = new URL(req.url ?? "/", `http://${req.headers.host ?? "localhost"}`);
  if (req.method === "GET" && url.pathname === "/health") {
    sendJson(res, 200, { status: "ok" });
    return;
  }

  if (url.pathname.startsWith("/worker/")) {
    if (!authorized(req, WORKER_TOKEN)) {
      sendJson(res, 401, { error: "worker_token_invalid" });
      return;
    }
    await routeWorker(req, res, url);
    return;
  }

  if (!authorized(req, API_TOKEN)) {
    sendJson(res, 401, { error: "unauthorized" });
    return;
  }
  await routeConsumer(req, res, url);
}

async function routeConsumer(req, res, url) {
  await expireTerminalTasks();

  if (req.method === "GET" && url.pathname === "/v1/status") {
    sendJson(res, 200, statusPayload());
    return;
  }

  if (req.method === "POST" && url.pathname === "/v1/tasks") {
    const body = await readJson(req);
    const task = submitTask(body);
    await saveStore();
    sendJson(res, 202, taskResponse(task));
    return;
  }

  const taskMatch = url.pathname.match(/^\/v1\/tasks\/([^/]+)$/);
  if (taskMatch && req.method === "GET") {
    const task = store.tasks[taskMatch[1]];
    if (!task) {
      sendJson(res, 404, { error: "task_not_found" });
      return;
    }
    sendJson(res, 200, taskResponse(task));
    return;
  }

  const cancelMatch = url.pathname.match(/^\/v1\/tasks\/([^/]+)\/cancel$/);
  if (cancelMatch && req.method === "POST") {
    const task = store.tasks[cancelMatch[1]];
    if (!task) {
      sendJson(res, 404, { error: "task_not_found" });
      return;
    }
    if (["completed", "failed"].includes(task.status)) {
      sendJson(res, 409, { error: "task_terminal", status: task.status });
      return;
    }
    if (task.status !== "cancelled") {
      task.status = "cancelled";
      task.completed_at = nowIso();
      task.expires_at = retentionIso();
      task.updated_at = nowIso();
      await saveStore();
    }
    sendJson(res, 200, taskResponse(task));
    return;
  }

  sendJson(res, 404, { error: "not_found" });
}

async function routeWorker(req, res, url) {
  await expireTerminalTasks();
  requeueExpiredLeases();

  if (req.method === "POST" && url.pathname === "/worker/task") {
    const worker = url.searchParams.get("worker") ?? "";
    if (!validWorkerLabel(worker)) {
      sendJson(res, 400, { error: "invalid_worker_label" });
      return;
    }
    const body = await readJson(req);
    const capabilities = sanitizeCapabilities(body.capabilities ?? {});
    const existing = Object.values(store.tasks).find(
      (task) => task.status === "dispatched" && task.assigned_worker === worker,
    );
    if (existing) {
      refreshLease(existing);
      upsertWorker(worker, capabilities, existing.id);
      await saveStore();
      sendJson(res, 200, { status: "task", task: workerTaskPayload(existing, worker) });
      return;
    }

    upsertWorker(worker, capabilities, null);
    const task = nextTaskFor(capabilities);
    if (!task) {
      await saveStore();
      sendJson(res, 200, { status: "idle" });
      return;
    }
    task.status = "dispatched";
    task.assigned_worker = worker;
    task.dispatched_at = nowIso();
    task.lease_expires_at = leaseIso();
    task.phase = "dispatched";
    task.phase_at = nowIso();
    task.updated_at = nowIso();
    upsertWorker(worker, capabilities, task.id);
    await saveStore();
    sendJson(res, 200, { status: "task", task: workerTaskPayload(task, worker) });
    return;
  }

  if (req.method === "POST" && url.pathname === "/worker/ack") {
    const body = await readJson(req);
    const task = store.tasks[body.task_id];
    const worker = String(body.worker ?? "");
    if (!validWorkerLabel(worker)) {
      sendJson(res, 400, { error: "invalid_worker_label" });
      return;
    }
    upsertWorker(worker, sanitizeCapabilities(body.capabilities ?? {}), body.task_id);
    if (!task || task.status !== "dispatched" || task.assigned_worker !== worker) {
      await saveStore();
      sendJson(res, 200, { status: "cancelled" });
      return;
    }
    task.lease_expires_at = leaseIso();
    if (typeof body.phase === "string") {
      task.phase = truncate(body.phase, 80);
      task.phase_at = nowIso();
    }
    if (typeof body.phase_detail === "string") {
      task.phase_detail = truncate(body.phase_detail, 500);
    }
    task.updated_at = nowIso();
    await saveStore();
    sendJson(res, 200, { status: "ok" });
    return;
  }

  if (req.method === "POST" && url.pathname === "/worker/result") {
    const body = await readJson(req);
    const task = store.tasks[body.task_id];
    const worker = String(body.worker ?? "");
    if (!task || task.status !== "dispatched" || task.assigned_worker !== worker) {
      sendJson(res, 200, { status: "ignored" });
      return;
    }
    if (typeof body.failure_reason === "string" && body.failure_reason.trim()) {
      task.status = "failed";
      task.failure_reason = truncate(body.failure_reason, 500);
    } else {
      task.status = "completed";
      task.output = body.output ?? null;
    }
    task.completed_at = nowIso();
    task.expires_at = retentionIso();
    task.lease_expires_at = null;
    task.updated_at = nowIso();
    if (store.workers[worker]) {
      store.workers[worker].current_task_id = null;
      store.workers[worker].last_seen_at = nowIso();
    }
    await saveStore();
    sendJson(res, 200, { status: task.status === "failed" ? "saved_failed" : "saved" });
    return;
  }

  sendJson(res, 404, { error: "not_found" });
}

function submitTask(body) {
  const kind = typeof body.kind === "string" && body.kind.trim() ? body.kind.trim() : "chat_completion";
  const model = String(body.model ?? "").trim();
  if (!model) {
    throw httpError(400, "model_required");
  }
  const clientRef = typeof body.client_ref === "string" && body.client_ref.trim()
    ? body.client_ref.trim()
    : null;
  if (clientRef && store.client_refs[clientRef]) {
    return store.tasks[store.client_refs[clientRef]];
  }
  const task = {
    id: randomUUID(),
    kind: truncate(kind, 64),
    model: truncate(model, 160),
    input: body.input ?? {},
    priority: Number.isFinite(body.priority) ? Math.trunc(body.priority) : 0,
    client_ref: clientRef,
    status: "queued",
    phase: null,
    phase_detail: null,
    phase_at: null,
    assigned_worker: null,
    dispatched_at: null,
    lease_expires_at: null,
    output: null,
    failure_reason: null,
    completed_at: null,
    expires_at: null,
    created_at: nowIso(),
    updated_at: nowIso(),
  };
  store.tasks[task.id] = task;
  if (clientRef) {
    store.client_refs[clientRef] = task.id;
  }
  return task;
}

function nextTaskFor(capabilities) {
  const models = normalizeModels(capabilities.models ?? []);
  const acceptsAny = models.includes("*");
  const candidates = Object.values(store.tasks).filter((task) => {
    if (task.status !== "queued") return false;
    if (acceptsAny) return true;
    if (models.length === 0) return false;
    return models.includes(task.model);
  });
  candidates.sort((a, b) => {
    if (b.priority !== a.priority) return b.priority - a.priority;
    return a.created_at.localeCompare(b.created_at);
  });
  return candidates[0] ?? null;
}

function requeueExpiredLeases() {
  const now = Date.now();
  for (const task of Object.values(store.tasks)) {
    if (
      task.status === "dispatched" &&
      task.lease_expires_at &&
      Date.parse(task.lease_expires_at) < now
    ) {
      task.status = "queued";
      task.assigned_worker = null;
      task.dispatched_at = null;
      task.lease_expires_at = null;
      task.phase = "requeued_after_lease_expiry";
      task.updated_at = nowIso();
    }
  }
}

function refreshLease(task) {
  task.lease_expires_at = leaseIso();
  task.updated_at = nowIso();
}

function upsertWorker(worker, capabilities, currentTaskId) {
  const existing = store.workers[worker] ?? { first_seen_at: nowIso() };
  store.workers[worker] = {
    ...existing,
    worker_label: worker,
    host_kind: stringOrNull(capabilities.host_kind),
    gpu_name: stringOrNull(capabilities.gpu_name),
    backend: stringOrNull(capabilities.backend),
    models: normalizeModels(capabilities.models ?? []),
    max_concurrency: positiveInt(capabilities.max_concurrency, 1),
    current_inflight: positiveInt(capabilities.current_inflight, currentTaskId ? 1 : 0),
    current_task_id: currentTaskId,
    worker_version: stringOrNull(capabilities.worker_version),
    last_seen_at: nowIso(),
  };
}

function statusPayload() {
  const queued = Object.values(store.tasks).filter((task) => task.status === "queued").length;
  const dispatched = Object.values(store.tasks).filter((task) => task.status === "dispatched").length;
  const recent = Date.now() - 120_000;
  const workers = Object.values(store.workers).filter(
    (worker) => Date.parse(worker.last_seen_at) >= recent,
  );
  return { queued, dispatched, active_workers: workers };
}

function taskResponse(task) {
  return {
    task_id: task.id,
    kind: task.kind,
    model: task.model,
    priority: task.priority,
    status: task.status,
    phase: task.phase,
    phase_detail: task.phase_detail,
    assigned_worker: task.assigned_worker,
    output: task.output,
    failure_reason: task.failure_reason,
    created_at: task.created_at,
    updated_at: task.updated_at,
    completed_at: task.completed_at,
  };
}

function workerTaskPayload(task, worker) {
  return {
    task_id: task.id,
    kind: task.kind,
    model: task.model,
    input: task.input,
    priority: task.priority,
    assigned_worker: worker,
    submitted_at: task.created_at,
  };
}

async function readJson(req) {
  const chunks = [];
  let size = 0;
  for await (const chunk of req) {
    size += chunk.length;
    if (size > MAX_BODY_BYTES) {
      throw httpError(413, "payload_too_large");
    }
    chunks.push(chunk);
  }
  if (chunks.length === 0) return {};
  try {
    return JSON.parse(Buffer.concat(chunks).toString("utf8"));
  } catch {
    throw httpError(400, "invalid_json");
  }
}

async function loadStore() {
  try {
    const raw = await readFile(STORE_PATH, "utf8");
    const parsed = JSON.parse(raw);
    return {
      tasks: parsed.tasks ?? {},
      workers: parsed.workers ?? {},
      client_refs: parsed.client_refs ?? {},
    };
  } catch {
    return { tasks: {}, workers: {}, client_refs: {} };
  }
}

async function saveStore() {
  const tmp = `${STORE_PATH}.tmp`;
  await writeFile(tmp, `${JSON.stringify(store, null, 2)}\n`, { mode: 0o600 });
  await rename(tmp, STORE_PATH);
}

async function expireTerminalTasks() {
  const now = Date.now();
  let changed = false;
  for (const [id, task] of Object.entries(store.tasks)) {
    if (task.expires_at && Date.parse(task.expires_at) < now) {
      delete store.tasks[id];
      if (task.client_ref) delete store.client_refs[task.client_ref];
      changed = true;
    }
  }
  if (changed) await saveStore();
}

function authorized(req, token) {
  if (DEV_INSECURE) return true;
  const auth = req.headers.authorization ?? "";
  if (!auth.startsWith("Bearer ")) return false;
  return safeEqual(auth.slice("Bearer ".length), token);
}

function safeEqual(a, b) {
  const left = Buffer.from(hash(a));
  const right = Buffer.from(hash(b));
  return timingSafeEqual(left, right);
}

function hash(value) {
  return createHash("sha256").update(String(value)).digest("hex");
}

function sendJson(res, status, payload) {
  const body = JSON.stringify(payload);
  res.writeHead(status, {
    "content-type": "application/json",
    "content-length": Buffer.byteLength(body),
  });
  res.end(body);
}

function normalizeModels(models) {
  const out = [];
  for (const model of Array.isArray(models) ? models.slice(0, 200) : []) {
    const normalized = String(model).trim().slice(0, 160);
    if (normalized && !out.includes(normalized)) out.push(normalized);
  }
  return out;
}

function sanitizeCapabilities(input) {
  return {
    host_kind: stringOrNull(input.host_kind),
    gpu_name: stringOrNull(input.gpu_name),
    backend: stringOrNull(input.backend),
    models: normalizeModels(input.models ?? []),
    max_concurrency: positiveInt(input.max_concurrency, 1),
    current_inflight: positiveInt(input.current_inflight, 0),
    worker_version: stringOrNull(input.worker_version),
  };
}

function stringOrNull(value) {
  return typeof value === "string" && value.trim() ? value.trim().slice(0, 160) : null;
}

function positiveInt(value, fallback) {
  const parsed = Number.parseInt(String(value ?? ""), 10);
  return Number.isFinite(parsed) && parsed >= 0 ? parsed : fallback;
}

function validWorkerLabel(label) {
  return /^[A-Za-z0-9_-]{1,64}$/.test(label);
}

function truncate(value, max) {
  return String(value).slice(0, max);
}

function nowIso() {
  return new Date().toISOString();
}

function leaseIso() {
  return new Date(Date.now() + TASK_TIMEOUT_SECS * 1000).toISOString();
}

function retentionIso() {
  return new Date(Date.now() + RETENTION_DAYS * 86_400_000).toISOString();
}

function httpError(status, code) {
  const err = new Error(code);
  err.status = status;
  return err;
}

process.on("uncaughtException", (err) => {
  if (err?.status) return;
  console.error(err);
  process.exit(1);
});
