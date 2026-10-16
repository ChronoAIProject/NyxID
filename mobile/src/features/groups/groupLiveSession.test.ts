import assert from "node:assert/strict";
import { test } from "node:test";
import { QueryClient, type QueryKey } from "@tanstack/react-query";
import {
  createGroupLiveSession,
  type GroupLiveRunOptions,
} from "./groupLiveSession";

const GROUP = "nyxg-0123456789abcdef0123456789abcdef";

function harness() {
  let now = 0;
  let next = 0;
  let allowed = true;
  let epoch = 1;
  const jobs = new Map<number, { at: number; callback: () => void }>();
  const cancelled: (() => void)[] = [];
  const runs: GroupLiveRunOptions[] = [];
  const rejectors: ((cause: unknown) => void)[] = [];
  const invalidations: { key: QueryKey; cancelRefetch?: boolean }[] = [];
  const client = new QueryClient();
  const invalidate = client.invalidateQueries.bind(client);
  client.invalidateQueries = async (filters, options) => {
    invalidations.push({
      key: filters?.queryKey ?? [],
      cancelRefetch: options?.cancelRefetch,
    });
    return invalidate(filters, options);
  };
  const session = createGroupLiveSession({
    available: (owner) => allowed && owner === epoch,
    run: (options) => {
      runs.push(options);
      return new Promise<void>((_resolve, reject) => rejectors.push(reject));
    },
    schedule: (callback, milliseconds) => {
      const id = ++next;
      jobs.set(id, { at: now + milliseconds, callback });
      return id;
    },
    cancel: (handle) => {
      const job = jobs.get(handle as number);
      if (job) cancelled.push(job.callback);
      jobs.delete(handle as number);
    },
  });
  function advance(milliseconds: number) {
    const until = now + milliseconds;
    for (;;) {
      const due = [...jobs].filter(([, job]) => job.at <= until);
      due.sort((left, right) => left[1].at - right[1].at);
      const first = due[0];
      if (!first) break;
      now = first[1].at;
      jobs.delete(first[0]);
      first[1].callback();
    }
    now = until;
  }
  return {
    session,
    client,
    runs,
    rejectors,
    invalidations,
    jobs,
    cancelled,
    advance,
    allow(value: boolean) {
      allowed = value;
    },
    epoch(value: number) {
      epoch = value;
    },
  };
}

test("consumers share one stream and only the last release aborts and resets status", () => {
  const h = harness();
  let notifications = 0;
  const unsubscribe = h.session.subscribe(() => notifications++);
  const available = () => true;
  const releaseFirst = h.session.acquire(h.client, 1, available);
  const releaseSecond = h.session.acquire(h.client, 1, available);
  assert.equal(h.runs.length, 1);
  assert.equal(h.session.getStatus(h.client, 1), "connecting");
  h.runs[0]!.onStatus("live");
  assert.equal(h.session.getStatus(h.client, 1), "live");
  assert.equal(notifications, 2);
  releaseFirst();
  releaseFirst();
  assert.equal(h.runs[0]!.signal.aborted, false);
  assert.equal(h.session.getStatus(h.client, 1), "live");
  releaseSecond();
  assert.equal(h.runs[0]!.signal.aborted, true);
  assert.equal(h.session.getStatus(h.client, 1), "idle");
  assert.equal(notifications, 3);
  unsubscribe();
  const releaseThird = h.session.acquire(h.client, 1, available);
  h.runs[1]!.onStatus("live");
  assert.equal(notifications, 3);
  releaseThird();
});

test("last release cancels both lanes, rejects late callbacks, and resets seen metadata", () => {
  const h = harness();
  const release = h.session.acquire(h.client, 1, () => true);
  const event = {
    type: "conversation" as const,
    id: "member-thread",
    group_id: GROUP,
    turn_id: "turn",
    messages: 2,
  };
  h.runs[0]!.onEvent(event);
  assert.equal(h.jobs.size, 2);
  release();
  assert.equal(h.jobs.size, 0);
  for (const callback of h.cancelled) callback();
  h.runs[0]!.onEvent({ type: "group", id: GROUP });
  h.runs[0]!.onStatus("live");
  h.advance(2500);
  assert.deepEqual(h.invalidations, []);
  assert.equal(h.session.getStatus(h.client, 1), "idle");
  const freshRelease = h.session.acquire(h.client, 1, () => true);
  h.runs[1]!.onEvent(event);
  h.advance(2000);
  assert.deepEqual(
    h.invalidations.map((call) => call.key),
    [
      ["workspace", "groups", 1, "messages", GROUP],
      ["workspace", "groups", 1, "list"],
      ["workspace", "agents"],
    ],
  );
  freshRelease();
});

test("a new epoch replaces the previous session and stale releases cannot stop it", () => {
  const h = harness();
  const releaseOld = h.session.acquire(h.client, 1, () => true);
  h.runs[0]!.onEvent({ type: "group", id: GROUP });
  h.epoch(2);
  const releaseNew = h.session.acquire(h.client, 2, () => true);
  assert.equal(h.runs[0]!.signal.aborted, true);
  releaseOld();
  assert.equal(h.runs[1]!.signal.aborted, false);
  h.runs[0]!.onStatus("live");
  h.runs[0]!.onEvent({ type: "ready" });
  for (const callback of h.cancelled) callback();
  h.runs[1]!.onStatus("live");
  h.runs[1]!.onEvent({ type: "group", id: GROUP });
  h.advance(2000);
  assert.equal(h.session.getStatus(h.client, 1), "idle");
  assert.equal(h.session.getStatus(h.client, 2), "live");
  assert.deepEqual(
    h.invalidations.map((call) => call.key),
    [
      ["workspace", "groups", 2, "messages", GROUP],
      ["workspace", "groups", 2, "list"],
    ],
  );
  releaseNew();
});

test("a new query client replaces the stream even within the same epoch", () => {
  const h = harness();
  const otherClient = new QueryClient();
  const releaseOld = h.session.acquire(h.client, 1, () => true);
  const releaseNew = h.session.acquire(otherClient, 1, () => true);
  assert.equal(h.runs[0]!.signal.aborted, true);
  assert.equal(h.session.getStatus(h.client, 1), "idle");
  assert.equal(h.session.getStatus(otherClient, 1), "connecting");
  releaseOld();
  assert.equal(h.runs[1]!.signal.aborted, false);
  releaseNew();
});

test("global and consumer availability fence events, statuses and delayed invalidation", () => {
  const h = harness();
  let foreground = true;
  const release = h.session.acquire(h.client, 1, () => foreground);
  h.allow(false);
  h.runs[0]!.onStatus("live");
  h.runs[0]!.onEvent({ type: "group", id: GROUP });
  assert.equal(h.session.getStatus(h.client, 1), "connecting");
  assert.equal(h.jobs.size, 0);
  h.allow(true);
  foreground = false;
  h.runs[0]!.onEvent({ type: "group", id: GROUP });
  assert.equal(h.jobs.size, 0);
  foreground = true;
  h.runs[0]!.onEvent({ type: "group", id: GROUP });
  h.allow(false);
  h.advance(2000);
  assert.deepEqual(h.invalidations, []);
  h.allow(true);
  h.runs[0]!.onEvent({ type: "group", id: GROUP });
  foreground = false;
  h.advance(2000);
  assert.deepEqual(h.invalidations, []);
  const releaseVisible = h.session.acquire(h.client, 1, () => true);
  h.runs[0]!.onStatus("live");
  h.runs[0]!.onEvent({ type: "group", id: GROUP });
  h.advance(150);
  assert.equal(h.session.getStatus(h.client, 1), "live");
  assert.equal(h.invalidations.length, 1);
  release();
  releaseVisible();
});

test("unavailable acquisition does not start transport; an available consumer starts it once", () => {
  const h = harness();
  h.allow(false);
  const releaseUnavailable = h.session.acquire(h.client, 1, () => true);
  assert.equal(h.runs.length, 0);
  assert.equal(h.session.getStatus(h.client, 1), "idle");
  h.allow(true);
  const releaseHidden = h.session.acquire(h.client, 1, () => false);
  // The original consumer is now globally available.
  assert.equal(h.runs.length, 1);
  const releaseVisible = h.session.acquire(h.client, 1, () => true);
  assert.equal(h.runs.length, 1);
  releaseUnavailable();
  releaseHidden();
  assert.equal(h.runs[0]!.signal.aborted, false);
  releaseVisible();
  assert.equal(h.runs[0]!.signal.aborted, true);

  const releaseOnlyHidden = h.session.acquire(h.client, 1, () => false);
  assert.equal(h.runs.length, 1);
  const releaseFirstVisible = h.session.acquire(h.client, 1, () => true);
  assert.equal(h.runs.length, 2);
  releaseOnlyHidden();
  releaseFirstVisible();
});

test("bursts coalesce by key at 150ms and throttle lists for 2000ms without cancelling reads", () => {
  const h = harness();
  const release = h.session.acquire(h.client, 1, () => true);
  for (let index = 0; index < 20; index++)
    h.runs[0]!.onEvent({ type: "group", id: GROUP });
  assert.equal(h.jobs.size, 2);
  h.advance(149);
  assert.equal(h.invalidations.length, 0);
  h.advance(1);
  assert.deepEqual(
    h.invalidations.map((call) => call.key),
    [["workspace", "groups", 1, "messages", GROUP]],
  );
  h.runs[0]!.onEvent({ type: "group", id: GROUP });
  h.advance(150);
  assert.equal(h.invalidations.length, 2);
  h.advance(1699);
  assert.equal(h.invalidations.length, 2);
  h.advance(1);
  assert.deepEqual(h.invalidations[2]!.key, ["workspace", "groups", 1, "list"]);
  assert.ok(h.invalidations.every((call) => call.cancelRefetch === false));
  release();
});

test("ready and resync catch up from saved group reads and clear coalesced signatures", () => {
  const h = harness();
  const release = h.session.acquire(h.client, 1, () => true);
  const event = {
    type: "conversation" as const,
    id: "member-thread",
    group_id: GROUP,
    turn_id: "turn",
    messages: 1,
  };
  for (const type of ["ready", "resync"] as const) {
    h.runs[0]!.onEvent(event);
    h.advance(2000);
    h.invalidations.length = 0;
    h.runs[0]!.onEvent({ type });
    h.advance(150);
    assert.deepEqual(
      h.invalidations.map((call) => call.key),
      [
        ["workspace", "groups", 1],
        ["workspace", "agents"],
      ],
    );
    h.invalidations.length = 0;
    h.runs[0]!.onEvent(event);
    h.advance(2000);
    assert.ok(
      h.invalidations.some((call) => call.key[3] === "list"),
      "first metadata after catchup must refresh the group list again",
    );
  }
  release();
});

test("more than 256 pending groups collapse to one epoch root without losing coverage", () => {
  const h = harness();
  const release = h.session.acquire(h.client, 1, () => true);
  for (let index = 0; index < 1000; index++)
    h.runs[0]!.onEvent({ type: "group", id: `group-${index}` });
  assert.equal(h.jobs.size, 2);
  h.advance(150);
  assert.deepEqual(
    h.invalidations.map((call) => call.key),
    [["workspace", "groups", 1]],
  );
  h.invalidations.length = 0;
  h.runs[0]!.onEvent({ type: "ready" });
  h.runs[0]!.onEvent({ type: "group", id: "group-after-ready" });
  h.advance(150);
  assert.deepEqual(
    h.invalidations.map((call) => call.key),
    [
      ["workspace", "groups", 1],
      ["workspace", "agents"],
    ],
  );
  release();
});

test("an unexpectedly rejected runner exposes connecting fallback without an unhandled rejection", async () => {
  const h = harness();
  const release = h.session.acquire(h.client, 1, () => true);
  h.runs[0]!.onStatus("live");
  h.rejectors[0]!(new Error("unexpected runner failure"));
  await Promise.resolve();
  assert.equal(h.session.getStatus(h.client, 1), "connecting");
  release();
  h.runs[0]!.onStatus("live");
  assert.equal(h.session.getStatus(h.client, 1), "idle");
});
