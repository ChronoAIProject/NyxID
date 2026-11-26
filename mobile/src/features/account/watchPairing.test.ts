import assert from "node:assert/strict";
import test from "node:test";
import type {
  IssuedLoginCode,
  LoginCodeStatus,
} from "../../lib/api/loginCodeApi";
import {
  displayWatchCode,
  pairingMessage,
  sendFailure,
  signOutMessage,
  watchAvailability,
  watchSetupCode,
  WatchPairingFlow,
  type WatchPairingState,
} from "./watchPairing";

const issued: IssuedLoginCode = {
  request_id: "req-1",
  code: "ABCD-EFGH",
  expires_at: "2026-11-16T12:05:00.000Z",
};

function status(value: LoginCodeStatus["status"]): LoginCodeStatus {
  return {
    request_id: "req-1",
    status: value,
    expires_at: issued.expires_at,
    redeemed_at: value === "redeemed" ? "2026-11-16T12:01:00Z" : null,
    client_label: value === "redeemed" ? "Apple Watch" : null,
    client_ip: null,
    client_ip_attribution: "none",
    can_revoke: value === "redeemed",
    auth_kind: "account_session",
  };
}

const tick = () => new Promise((resolve) => setTimeout(resolve, 5));

async function until(check: () => boolean) {
  for (let i = 0; i < 200 && !check(); i++) await tick();
  assert.ok(check(), "condition never became true");
}

function harness(overrides: {
  mint?: () => Promise<IssuedLoginCode>;
  statuses?: LoginCodeStatus["status"][];
  send?: (message: Record<string, string>) => Promise<"delivered" | "queued">;
  now?: () => number;
}) {
  const states: WatchPairingState[] = [];
  const sent: Record<string, string>[] = [];
  const cancelled: string[] = [];
  const cleared: number[] = [];
  const statuses = [...(overrides.statuses ?? ["redeemed"])];
  const flow = new WatchPairingFlow({
    api: {
      mint: overrides.mint ?? (async () => issued),
      status: async () => status(statuses.length > 1 ? statuses.shift()! : statuses[0]!),
      cancel: async (id) => {
        cancelled.push(id);
      },
    },
    link: {
      send:
        overrides.send ??
        (async (message) => {
          sent.push(message);
          return "delivered";
        }),
      clearPending: async () => {
        cleared.push(Date.now());
      },
    },
    apiBaseUrl: () => "https://auth.example.com/api/v1/",
    notify: (state) => states.push(state),
    pollInterval: 1,
    now: overrides.now ?? (() => Date.parse("2026-11-16T12:00:00Z")),
  });
  return { flow, states, sent, cancelled, cleared };
}

test("the watch receives only a one-time code, its own setup code and the addresses", () => {
  assert.deepEqual(
    pairingMessage(issued, "https://auth.example.com/api/v1/", "482913", "https://app.example.com/"),
    {
      v: "1",
      kind: "pair",
      code: "ABCD-EFGH",
      api_base_url: "https://auth.example.com/api/v1",
      match: "482913",
      frontend_url: "https://app.example.com",
      expires_at: "2026-11-16T12:05:00.000Z",
    },
  );
  assert.equal(pairingMessage(issued, "https://a.example/api/v1", "000111").frontend_url, undefined);
  assert.deepEqual(signOutMessage(), { v: "1", kind: "sign_out" });
});

test("the watch's setup code is shown as it appears on the watch", () => {
  const base = { supported: true, activated: true, paired: true, installed: true, reachable: true };
  assert.equal(watchSetupCode({ ...base, watchCode: "482913" }), "482913");
  assert.equal(watchSetupCode({ ...base, watchCode: "" }), null);
  assert.equal(watchSetupCode({ ...base, watchCode: "48291" }), null);
  assert.equal(watchSetupCode({ ...base }), null);
  assert.equal(watchSetupCode(null), null);
  assert.equal(displayWatchCode("482913"), "482 913");
});

test("availability follows WatchConnectivity", () => {
  const base = { supported: true, activated: true, paired: true, installed: true, reachable: false };
  assert.equal(watchAvailability(null), "checking");
  assert.equal(watchAvailability({ ...base, supported: false }), "unsupported");
  assert.equal(watchAvailability({ ...base, paired: false }), "no_watch");
  assert.equal(watchAvailability({ ...base, activated: false }), "no_watch");
  assert.equal(watchAvailability({ ...base, installed: false }), "not_installed");
  assert.equal(watchAvailability(base), "ready");
});

test("mint, send, then follow the code until the watch redeems it", async () => {
  const { flow, states, sent, cancelled } = harness({
    statuses: ["pending", "pending", "redeemed"],
  });
  await flow.start("482913");
  await until(() => flow.current.step === "connected");
  assert.deepEqual(
    states.map((state) => state.step),
    ["minting", "sending", "waiting", "connected"],
  );
  assert.equal(sent.length, 1);
  assert.equal(sent[0]?.code, "ABCD-EFGH");
  assert.equal(sent[0]?.match, "482913");
  assert.deepEqual(cancelled, []);
});

test("a queued delivery is reported so the person can open the watch app", async () => {
  const { flow, states, cleared } = harness({
    statuses: ["pending"],
    send: async () => "queued",
  });
  await flow.start("482913");
  assert.deepEqual(states.at(-1), { step: "waiting", delivery: "queued", requestId: "req-1" });
  assert.equal(cleared.length, 0);
  flow.stop();
  // The unused code doesn't stay queued for the watch.
  assert.equal(cleared.length, 1);
});

test("a queued code is cleared once the watch has used it", async () => {
  const { flow, cleared } = harness({
    statuses: ["pending", "redeemed"],
    send: async () => "queued",
  });
  await flow.start("482913");
  await until(() => flow.current.step === "connected");
  assert.equal(cleared.length, 1);
  flow.stop();
  assert.equal(cleared.length, 1, "nothing left to clear after success");
});

test("a code delivered directly leaves nothing queued to clear", async () => {
  const { flow, cleared } = harness({ statuses: ["redeemed"] });
  await flow.start("482913");
  await until(() => flow.current.step === "connected");
  flow.stop();
  assert.equal(cleared.length, 0);
});

test("leaving before the watch signs in cancels the unused code", async () => {
  const { flow, cancelled } = harness({ statuses: ["pending"] });
  await flow.start("482913");
  flow.stop();
  await tick();
  assert.deepEqual(cancelled, ["req-1"]);
  assert.equal(flow.current.step, "idle");
});

test("a send failure cancels the code and explains what to do", async () => {
  const { flow, cancelled } = harness({
    send: async () => {
      throw Object.assign(new Error("not installed"), { code: "ERR_WATCH_APP_NOT_INSTALLED" });
    },
  });
  await flow.start("482913");
  await tick();
  assert.deepEqual(cancelled, ["req-1"]);
  assert.deepEqual(flow.current, {
    step: "failed",
    message: "Install NyxID on your Apple Watch from the Watch app, then try again.",
  });
});

test("an expired code fails instead of waiting forever", async () => {
  const { flow } = harness({ statuses: ["expired"] });
  await flow.start("482913");
  await until(() => flow.current.step === "failed");
  assert.match((flow.current as { message: string }).message, /expired/);
});

test("a code that outlives its expiry locally is cancelled", async () => {
  const { flow, cancelled } = harness({
    statuses: ["pending"],
    now: () => Date.parse("2026-11-16T12:10:00Z"),
  });
  await flow.start("482913");
  await until(() => flow.current.step === "failed");
  assert.deepEqual(cancelled, ["req-1"]);
});

test("an MFA step-up refusal is handed to the caller", async () => {
  const handled: unknown[] = [];
  const refusal = Object.assign(new Error("verify"), { errorKey: "mfa_step_up_required" });
  const states: WatchPairingState[] = [];
  const flow = new WatchPairingFlow({
    api: {
      mint: async () => {
        throw refusal;
      },
      status: async () => status("pending"),
      cancel: async () => {},
    },
    link: { send: async () => "delivered", clearPending: async () => {} },
    apiBaseUrl: () => "https://a.example/api/v1",
    notify: (state) => states.push(state),
    handleMintError: async (error) => {
      handled.push(error);
      return true;
    },
  });
  await flow.start("482913");
  assert.deepEqual(handled, [refusal]);
  assert.equal(flow.current.step, "idle");
});

test("send failures map native codes to guidance", () => {
  assert.equal(
    sendFailure({ code: "ERR_WATCH_NOT_PAIRED" }),
    "No Apple Watch is paired with this iPhone.",
  );
  assert.match(sendFailure(new Error("boom")), /Open NyxID on your watch/);
});
