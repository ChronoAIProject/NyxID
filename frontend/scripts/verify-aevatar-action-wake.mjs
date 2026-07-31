import { randomUUID } from "node:crypto";

const baseUrl = requireEnvironment("NYXID_URL").replace(/\/+$/, "");
const accessToken = requireEnvironment("NYXID_ACCESS_TOKEN");
const actorId = requireEnvironment("NYXID_AEVATAR_ACTOR_ID");
const originTurnId = requireEnvironment("NYXID_AEVATAR_ORIGIN_TURN_ID");

function requireEnvironment(name) {
  const value = process.env[name]?.trim();
  if (!value) throw new Error(`${name} is required.`);
  return value;
}

function frameType(frame) {
  if (typeof frame.type === "string") return frame.type.toUpperCase();
  if (frame.runStarted) return "RUN_STARTED";
  if (frame.runFinished) return "RUN_FINISHED";
  if (frame.runStopped) return "RUN_STOPPED";
  if (frame.runError) return "RUN_ERROR";
  return "UNKNOWN";
}

function parseSse(payload) {
  return payload
    .split(/\r?\n\r?\n/)
    .map((event) =>
      event
        .split(/\r?\n/)
        .filter((line) => line.startsWith("data:"))
        .map((line) => line.slice(5).trimStart())
        .join("\n"),
    )
    .filter((data) => data && data !== "[DONE]")
    .map((data) => JSON.parse(data));
}

function problemMessage(payload) {
  try {
    const problem = JSON.parse(payload);
    return typeof problem.message === "string"
      ? problem.message
      : typeof problem.title === "string"
        ? problem.title
        : "The response did not include an error message.";
  } catch {
    return "The response was not JSON.";
  }
}

async function postSse(path, body) {
  const response = await fetch(`${baseUrl}${path}`, {
    method: "POST",
    headers: {
      Accept: "text/event-stream",
      Authorization: `Bearer ${accessToken}`,
      "Content-Type": "application/json",
    },
    body: JSON.stringify(body),
    signal: AbortSignal.timeout(240_000),
  });
  const payload = await response.text();
  if (!response.ok) {
    throw new Error(
      `${path} returned HTTP ${response.status}: ${problemMessage(payload)}`,
    );
  }
  if (!response.headers.get("content-type")?.includes("text/event-stream")) {
    throw new Error(`${path} did not return text/event-stream.`);
  }
  const frames = parseSse(payload);
  if (frames.length === 0) throw new Error(`${path} returned no SSE frames.`);
  return frames;
}

function startedIdentity(frames, expectedActorId) {
  const start = frames.find((frame) => frameType(frame) === "RUN_STARTED");
  if (!start) throw new Error("The producer did not emit RUN_STARTED.");

  const actorId = start.actorId ?? start.runStarted?.actorId ?? expectedActorId;
  const turnId =
    start.turnId ?? start.runStarted?.turnId ?? start.runStarted?.runId;
  if (
    typeof actorId !== "string" ||
    !/^nyxid-chat-[A-Za-z0-9_-]{1,117}$/.test(actorId)
  ) {
    throw new Error("The producer did not identify a typed NyxIdChat actor.");
  }
  if (typeof turnId !== "string" || turnId.length === 0) {
    throw new Error("The producer did not identify the started turn.");
  }
  return { actorId, turnId };
}

if (!/^nyxid-chat-[A-Za-z0-9_-]{1,117}$/.test(actorId)) {
  throw new Error("NYXID_AEVATAR_ACTOR_ID must identify a typed NyxIdChat actor.");
}
if (
  originTurnId.length > 256 ||
  /[\s\u0000-\u001f\u007f/\\?#]/.test(originTurnId)
) {
  throw new Error("NYXID_AEVATAR_ORIGIN_TURN_ID is not a control identity.");
}

const wakeFrames = await postSse(
  "/api/v1/assistant/chat",
  {
    type: "action.continue",
    conversationId: actorId,
    clientRequestId: randomUUID(),
    originTurnId,
    actions: [],
  },
);
const continuation = startedIdentity(wakeFrames, actorId);
if (continuation.actorId !== actorId) {
  throw new Error("The wake continuation changed the authoritative actor.");
}
if (continuation.turnId === originTurnId) {
  throw new Error("The wake did not start a distinct continuation turn.");
}

console.log(
  `Aevatar accepted actions: [] for ${actorId} (${originTurnId} -> ${continuation.turnId}).`,
);
