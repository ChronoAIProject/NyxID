#!/usr/bin/env node
import { setTimeout as sleep } from "node:timers/promises";

const args = parseArgs(process.argv.slice(2));

const serviceUrl = required(args["service-url"], "--service-url").replace(/\/$/, "");
const worker = required(args.worker, "--worker");
const endpointUrl = required(args["endpoint-url"], "--endpoint-url");
const tokenEnv = args["token-env"] ?? "COMPUTE_POOL_WORKER_TOKEN";
const workerToken = process.env[tokenEnv];
if (!workerToken) {
  throw new Error(`${tokenEnv} is empty or unset`);
}
const backendToken = args["backend-token-env"] ? process.env[args["backend-token-env"]] : null;
const models = arrayArg(args.model);
const pollIntervalMs = seconds(args["poll-interval-secs"], 5) * 1000;
const ackIntervalMs = seconds(args["ack-interval-secs"], 30) * 1000;
const maxAckFailures = positiveInt(args["max-ack-failures"], 3);
const requestTimeoutMs = seconds(args["request-timeout-secs"], 14_400) * 1000;

console.error(`compute worker ${worker} polling ${serviceUrl}`);

for (;;) {
  const polled = await postJson(`${serviceUrl}/worker/task?worker=${encodeURIComponent(worker)}`, {
    capabilities: capabilities(0),
  });
  if (polled.status === "idle") {
    await sleep(pollIntervalMs);
    continue;
  }
  const task = polled.task;
  console.error(`claimed task ${task.task_id} kind=${task.kind} model=${task.model}`);
  try {
    const output = await executeWithHeartbeats(task);
    if (output.cancelled) {
      console.error(`task ${task.task_id} cancelled`);
      continue;
    }
    await postJson(`${serviceUrl}/worker/result`, {
      task_id: task.task_id,
      worker,
      output: output.value,
    });
    console.error(`completed task ${task.task_id}`);
  } catch (err) {
    const reason = err?.message ?? String(err);
    await postJson(`${serviceUrl}/worker/result`, {
      task_id: task.task_id,
      worker,
      failure_reason: reason,
    });
    console.error(`failed task ${task.task_id}: ${reason}`);
  }
}

async function executeWithHeartbeats(task) {
  const controller = new AbortController();
  const local = executeLocal(task, controller.signal).then(
    (value) => ({ value }),
    (err) => ({ failed: err }),
  );
  let done = false;
  let ackFailures = 0;
  const heartbeats = (async () => {
    while (!done) {
      await sleep(ackIntervalMs);
      if (done) return { cancelled: false };
      const ack = await postAck(task);
      if (ack.failed) {
        ackFailures += 1;
        console.error(
          `ack failed for task ${task.task_id} (${ackFailures}/${maxAckFailures}): ${ack.error}`,
        );
        if (ackFailures >= maxAckFailures) {
          controller.abort();
          return { failed: new Error(`ack failed ${ackFailures} times; local request aborted`) };
        }
        continue;
      }
      ackFailures = 0;
      if (ack.status === "cancelled") {
        controller.abort();
        return { cancelled: true };
      }
    }
    return { cancelled: false };
  })();

  const winner = await Promise.race([
    local,
    heartbeats,
  ]);
  done = true;
  if (winner.failed) throw winner.failed;
  if (winner.cancelled) return winner;
  return winner;
}

async function postAck(task) {
  try {
    return await postJson(`${serviceUrl}/worker/ack`, {
      task_id: task.task_id,
      worker,
      phase: "running",
      capabilities: capabilities(1),
    });
  } catch (err) {
    return { failed: true, error: err?.message ?? String(err) };
  }
}

async function executeLocal(task, signal) {
  const body = typeof task.input === "object" && task.input !== null ? { ...task.input } : {};
  if (!body.model) body.model = task.model;

  const timeout = AbortSignal.timeout(requestTimeoutMs);
  const combined = AbortSignal.any([signal, timeout]);
  const headers = { "content-type": "application/json" };
  if (backendToken) headers.authorization = `Bearer ${backendToken}`;

  const response = await fetch(endpointUrl, {
    method: "POST",
    headers,
    body: JSON.stringify(body),
    signal: combined,
  });
  const text = await response.text();
  if (!response.ok) {
    throw new Error(`local backend HTTP ${response.status}: ${text.slice(0, 500)}`);
  }
  return text ? JSON.parse(text) : null;
}

async function postJson(url, body) {
  const response = await fetch(url, {
    method: "POST",
    headers: {
      authorization: `Bearer ${workerToken}`,
      "content-type": "application/json",
    },
    body: JSON.stringify(body),
  });
  const text = await response.text();
  if (!response.ok) {
    throw new Error(`service HTTP ${response.status}: ${text.slice(0, 500)}`);
  }
  return text ? JSON.parse(text) : {};
}

function capabilities(currentInflight) {
  return {
    host_kind: args["host-kind"] ?? null,
    gpu_name: args["gpu-name"] ?? null,
    backend: args.backend ?? null,
    models,
    max_concurrency: Number.parseInt(args["max-concurrency"] ?? "1", 10),
    current_inflight: currentInflight,
    worker_version: "compute-pool-service-worker/0.1.0",
  };
}

function parseArgs(argv) {
  const out = {};
  for (let i = 0; i < argv.length; i += 1) {
    const item = argv[i];
    if (!item.startsWith("--")) continue;
    const key = item.slice(2);
    const next = argv[i + 1];
    if (!next || next.startsWith("--")) {
      out[key] = "true";
    } else if (out[key]) {
      out[key] = Array.isArray(out[key]) ? [...out[key], next] : [out[key], next];
      i += 1;
    } else {
      out[key] = next;
      i += 1;
    }
  }
  return out;
}

function arrayArg(value) {
  if (!value) return [];
  return Array.isArray(value) ? value : [value];
}

function required(value, name) {
  if (!value) throw new Error(`${name} is required`);
  return value;
}

function seconds(value, fallback) {
  const parsed = Number.parseInt(value ?? "", 10);
  return Number.isFinite(parsed) && parsed > 0 ? parsed : fallback;
}

function positiveInt(value, fallback) {
  const parsed = Number.parseInt(value ?? "", 10);
  return Number.isFinite(parsed) && parsed > 0 ? parsed : fallback;
}
