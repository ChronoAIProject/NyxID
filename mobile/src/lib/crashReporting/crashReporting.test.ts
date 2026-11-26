import {localizedError} from "../../i18n/localizedError";
import assert from "node:assert/strict";
import test from "node:test";
import { randomUUID } from "node:crypto";
import { parseMobileStack } from "./stack";
import { revalidateNativeRecord, type CrashBinding } from "./nativeRecord";
import {
  chainGlobalHandler,
  chainRejections,
  type ErrorHandler,
} from "./globalHandler";
import { createCrashLifecycle, type CrashNativeModule } from "./lifecycle";
import { buildApiError } from "../api/responseError";
import { createCrashRelay } from "./reporter";
import type { ConsentSnapshot } from "../analyticsConsentState";
import type { RelayRequest, AnalyticsPermission } from "../analyticsRelay";
const sentinel =
  "SECRET_SENTINEL Bearer private person@example.test eyJhbGci.jwt";
const report = () => ({
  schema_version: 1,
  event_id: randomUUID(),
  captured_at: new Date().toISOString(),
  platform: "ios",
  kind: "js_fatal",
  exception_class: "TypeError",
  app_version: "1.2.3",
  app_build: "123",
  os_version: "18.1",
  runtime: { name: "hermes", major: 0 },
  js_frames: [{ asset: "main.jsbundle", line: 12, column: 3 }],
  native_images: [],
  native_frames: [],
});
const bound = (): CrashBinding => ({
  accountId: randomUUID(),
  revision: randomUUID(),
  epochId: randomUUID(),
});
test("Hermes and JSC parser never retains message, cause, function, token, email, URL or absolute path", () => {
  const frames = parseMobileStack(
    `Error ${sentinel}\n at SECRET_FUNCTION (address at /Users/SECRET_USER/app/main.jsbundle:12:3)\nSECRET_FUNCTION@https://user:pass@evil.test/SECRET_URL?token=x:2:4\n at x (index.android.bundle:1:2)`,
  );
  assert.deepEqual(frames, [
    { asset: "main.jsbundle", line: 12, column: 3 },
    { asset: "<external>", line: 2, column: 4 },
    { asset: "index.android.bundle", line: 1, column: 2 },
  ]);
  for (const value of [
    "SECRET_",
    "Bearer",
    "person@",
    "eyJ",
    "/Users",
    "https:",
  ])
    assert.ok(!JSON.stringify(frames).includes(value));
});
test("native records reject reason registers message customData absolute paths and extra keys at every boundary", () => {
  const good = { binding: bound(), report: report() };
  assert.ok(revalidateNativeRecord(JSON.stringify(good)));
  for (const key of [
    "reason",
    "registers",
    "message",
    "customData",
    "absolutePath",
    "extra",
  ]) {
    assert.equal(
      revalidateNativeRecord(JSON.stringify({ ...good, [key]: sentinel })),
      null,
    );
    assert.equal(
      revalidateNativeRecord(
        JSON.stringify({
          ...good,
          report: { ...good.report, [key]: sentinel },
        }),
      ),
      null,
    );
    assert.equal(
      revalidateNativeRecord(
        JSON.stringify({
          ...good,
          binding: { ...good.binding, [key]: sentinel },
        }),
      ),
      null,
    );
  }
  assert.equal(
    revalidateNativeRecord(
      JSON.stringify({
        ...good,
        report: {
          ...good.report,
          js_frames: [
            { asset: "/Users/SECRET_USER/main.jsbundle", line: 1, column: 2 },
          ],
        },
      }),
    ),
    null,
  );
  assert.equal(
    revalidateNativeRecord(
      JSON.stringify({
        ...good,
        report: {
          ...good.report,
          runtime: { name: "hermes", major: 0, message: sentinel },
        },
      }),
    ),
    null,
  );
});
test("fatal handler persists synchronously before the original RN fatal handler and preserves rejection callbacks", () => {
  const calls: string[] = [];
  let handler: ErrorHandler = (_e, fatal) => {
    assert.equal(fatal, true);
    calls.push("previous");
  };
  const restore = chainGlobalHandler(
    {
      getGlobalHandler: () => handler,
      setGlobalHandler: (h) => {
        handler = h;
      },
    },
    () => calls.push("persist"),
    () => calls.push("nonfatal"),
  );
  handler(new Error(sentinel), true);
  assert.deepEqual(calls, ["persist", "previous"]);
  restore();
  const options = chainRejections(
    {
      onUnhandled: () => calls.push("rn-unhandled"),
      onHandled: () => calls.push("rn-handled"),
    },
    () => calls.push("report"),
    true,
  );
  options.onUnhandled!(1, new Error(sentinel));
  options.onHandled!(1);
  assert.deepEqual(calls.slice(2), ["report", "rn-unhandled", "rn-handled"]);
});
function harness() {
  let snapshot: ConsentSnapshot = {
    accountId: randomUUID(),
    revision: 1,
    enabled: true,
    asked: true,
    policy: 2,
    isHydrated: true,
    persistenceError: false,
  };
  let binding: CrashBinding | null = null,
    records: string[] = [],
    permission: AnalyticsPermission = {
      account_id: snapshot.accountId!,
      revision: randomUUID(),
      enabled: true,
      policy_version: "2",
      current_policy_version: "2",
      diagnostics_status: "available", diagnostics_available: true,
    };
  const sent: unknown[] = [];
  const fatal: string[] = [];
  let arms = 0,
    purges = 0, pauses = 0;
  let failure: unknown = null;
  let armed = false;
  const native: CrashNativeModule = {
    getAppIdentity: () => ({ version: "9.8.7", build: "654" }),
    getBinding: () => binding,
    arm: (b) => {
      arms++;
      binding = { ...b };
      armed = true;
      return true;
    },
    pause: () => { pauses++; armed = false; },
    disarm: () => {
      purges++;
      armed = false;
      binding = null;
      records = [];
    },
    recordFatalJs: (json) => {
      assert.ok(armed);
      fatal.push(json);
      records.push(JSON.stringify({ binding, report: JSON.parse(json) }));
    },
    collectPending: () => records,
    removeRecords: (ids) => {
      records = records.filter(
        (raw) => !ids.includes(JSON.parse(raw).report.event_id),
      );
    },
  };
  const request: RelayRequest = async <T>(
    path: string,
    opts?: Parameters<RelayRequest>[1],
  ) => {
    opts?.authorityGuard?.();
    if (path.endsWith("/analytics")) { if (failure) throw failure; return permission as T; }
    sent.push(opts?.body);
    return {} as T;
  };
  const create = () => createCrashLifecycle({
    native,
    snapshot: () => snapshot,
    request,
    uuid: randomUUID,
    platform: "ios",
    osVersion: () => "18.1",
  });
  const lifecycle = create();
  return {
    lifecycle,
    relaunch: create,
    sent,
    fatal,
    native,
    get binding() {
      return binding;
    },
    get records() {
      return records;
    },
    get arms() {
      return arms;
    },
    get pauses() { return pauses; },
    fail: (error: unknown) => { failure = error; },
    get purges() {
      return purges;
    },
    snapshot: () => snapshot,
    change: (patch: Partial<ConsentSnapshot>) => {
      snapshot = { ...snapshot, ...patch, revision: snapshot.revision + 1 };
    },
    permission: (patch: Partial<AnalyticsPermission>) => {
      permission = { ...permission, ...patch };
    },
  };
}
test("relaunch re-arms the same epoch and drains a closed fatal record with native installed version", async () => {
  const h = harness();
  await h.lifecycle.setRestoring(false);
  const epoch = h.binding!.epochId;
  const error = new TypeError(sentinel, { cause: new Error(sentinel) });
  error.stack = ` at SECRET_FUNCTION (address at /Users/SECRET_USER/main.jsbundle:12:3)`;
  h.lifecycle.fatal(error);
  assert.equal(h.fatal.length, 1);
  await h.lifecycle.refresh();
  assert.equal(h.binding!.epochId, epoch);
  assert.equal(h.arms, 2);
  assert.equal(h.records.length, 0);
  assert.equal(h.sent.length, 1);
  const body = h.sent[0] as {
    report: { app_version: string; app_build: string };
  };
  assert.equal(body.report.app_version, "9.8.7");
  assert.equal(body.report.app_build, "654");
  for (const value of [
    "SECRET_",
    "Bearer",
    "person@",
    "eyJ",
    "/Users",
    "message",
    "cause",
  ])
    assert.ok(!JSON.stringify(body).includes(value));
});
test("account A to B to A, withdrawal/regrant, kill switch and logout cannot resurrect records", async () => {
  const h = harness();
  await h.lifecycle.setRestoring(false);
  const a = h.snapshot().accountId!;
  const first = h.binding!.epochId;
  h.lifecycle.fatal(new Error(sentinel));
  const b = randomUUID();
  h.change({ accountId: b });
  h.permission({ account_id: b, revision: randomUUID() });
  await h.lifecycle.refresh();
  assert.equal(h.records.length, 0);
  h.change({ accountId: a });
  h.permission({ account_id: a, revision: randomUUID() });
  await h.lifecycle.refresh();
  assert.notEqual(h.binding!.epochId, first);
  assert.equal(h.sent.length, 0);
  h.lifecycle.fatal(new Error(sentinel));
  h.change({ enabled: false });
  await h.lifecycle.refresh();
  assert.equal(h.binding, null);
  assert.equal(h.records.length, 0);
  h.change({ enabled: true });
  h.permission({ revision: randomUUID() });
  await h.lifecycle.refresh();
  assert.equal(h.records.length, 0);
  h.lifecycle.fatal(new Error(sentinel));
  h.permission({ diagnostics_status: "not_offered", diagnostics_available: false });
  await h.lifecycle.refresh();
  assert.equal(h.binding, null);
  assert.equal(h.records.length, 0);
  h.change({ accountId: null });
  h.lifecycle.disarm();
  assert.equal(h.binding, null);
  assert.ok(h.purges >= 4);
});
test("policy one and unhydrated choices never arm", async () => {
  const h = harness();
  h.change({ policy: 1 });
  await h.lifecycle.setRestoring(false);
  assert.equal(h.arms, 0);
  h.change({ policy: 2, isHydrated: false });
  await h.lifecycle.refresh();
  assert.equal(h.arms, 0);
});

test("Android uses installed version and reports JavaScript fatal records only", async () => {
  const b = bound();
  const snapshot: ConsentSnapshot = {
    accountId: b.accountId,
    revision: 1,
    enabled: true,
    asked: true,
    policy: 2,
    isHydrated: true,
    persistenceError: false,
  };
  const calls: string[] = [];
  let saved: Record<string, unknown> | null = null;
  const lifecycle = createCrashLifecycle({
    native: {
      getAppIdentity: () => ({ version: "2.3.4", build: "99" }),
      getBinding: () => b,
      arm: () => { calls.push("arm"); return true; },
      pause: () => { calls.push("pause"); },
      disarm: () => calls.push("purge"),
      recordFatalJs: (json) => {
        saved = JSON.parse(json);
      },
      collectPending: () => [],
      removeRecords: () => {},
    },
    snapshot: () => snapshot,
    request: async <T>() =>
      ({
        account_id: b.accountId,
        revision: b.revision,
        enabled: true,
        policy_version: "2",
        diagnostics_status: "available", diagnostics_available: true,
      }) as T,
    uuid: randomUUID,
    platform: "android",
    osVersion: () => "35",
  });
  await lifecycle.setRestoring(false);
  lifecycle.fatal(new TypeError(sentinel));
  assert.equal(
    (saved as unknown as Record<string, unknown>).platform,
    "android",
  );
  assert.equal((saved as unknown as Record<string, unknown>).kind, "js_fatal");
  assert.equal(
    (saved as unknown as Record<string, unknown>).app_version,
    "2.3.4",
  );
  assert.ok(!JSON.stringify(saved).includes("SECRET_SENTINEL"));
});

test("device/session generations fence pending mobile sends even when the binding epoch is retained", async () => {
  const binding = bound();
  let generation = 1,
    denials = 0,
    posts = 0;
  let release!: (value: unknown) => void;
  const request: RelayRequest = async <T>(path: string) => {
    if (path.endsWith("/analytics"))
      return new Promise<T>((resolve) => {
        release = (value) => resolve(value as T);
      });
    posts++;
    return {} as T;
  };
  const send = createCrashRelay(
    request,
    () => true,
    () => {
      denials++;
    },
    () => generation,
  );
  const pending = send(
    revalidateNativeRecord(JSON.stringify({ binding, report: report() }))!
      .report,
    binding,
  );
  generation++;
  release({
    account_id: binding.accountId,
    enabled: true,
    policy_version: "2",
    diagnostics_status: "available", diagnostics_available: true,
    revision: binding.revision,
  });
  assert.equal(await pending, false);
  assert.equal(posts, 0);
  assert.equal(denials, 0);
});

test("a stale account denial cannot purge the successor mobile binding", async () => {
  const binding = bound();
  let generation = 1,
    denials = 0;
  let reject!: (error: unknown) => void;
  const request: RelayRequest = async <T>() =>
    new Promise<T>((_resolve, fail) => {
      reject = fail;
    });
  const send = createCrashRelay(
    request,
    () => true,
    () => {
      denials++;
    },
    () => generation,
  );
  const pending = send(
    revalidateNativeRecord(JSON.stringify({ binding, report: report() }))!
      .report,
    binding,
  );
  generation++;
  reject({ errorCode: 13401 });
  assert.equal(await pending, false);
  assert.equal(denials, 0);
});

test("offline relaunch pauses without purging and retries the same revision; denial purges", async () => {
  const h = harness();
  await h.lifecycle.setRestoring(false);
  h.lifecycle.fatal(new Error(sentinel));
  const epoch = h.binding!.epochId;
  const purges = h.purges;
  const relaunched = h.relaunch();
  h.fail(new Error("network unavailable"));
  await relaunched.setRestoring(false);
  assert.equal(h.purges, purges);
  assert.equal(h.records.length, 1);
  assert.equal(h.binding!.epochId, epoch);
  relaunched.fatal(new Error("while paused"));
  assert.equal(h.fatal.length, 1);
  assert.ok(h.pauses > 0);
  h.fail(null);
  await relaunched.refresh();
  assert.equal(h.binding!.epochId, epoch);
  assert.equal(h.records.length, 0);
  assert.equal(h.sent.length, 1);
  relaunched.fatal(new Error("next"));
  h.fail({ errorCode: 13400 });
  await relaunched.refresh();
  assert.equal(h.binding, null);
  assert.equal(h.records.length, 0);
});
test("mobile excludes HTTP refusals and reporter-owned errors but retains server failures", async () => {
  const { ApiError } = await import("../api/ApiError");
  const { crashAuthorityError } = await import("./reporter");
  const h = harness();
  await h.lifecycle.setRestoring(false);
  for (const statusCode of [0, 400, 401, 403, 404, 429, 499]) {
    await h.lifecycle.capture(new ApiError({statusCode, errorCode: 1000, errorKey: "refused", message: sentinel}));
  }
  await h.lifecycle.capture(crashAuthorityError());
  assert.equal(h.sent.length, 0);
  await h.lifecycle.capture(new ApiError({statusCode: 503, errorCode: 1000, errorKey: "unavailable", message: sentinel}));
  assert.equal(h.sent.length, 1);
  assert.ok(!JSON.stringify(h.sent).includes(sentinel));
  assert.ok(!JSON.stringify(h.sent).includes('"runtime"'));
});

test("an unavailable native reporter cannot arm JavaScript capture", async () => {
  const h=harness();
  h.native.arm=()=>false;
  await h.lifecycle.setRestoring(false);
  h.lifecycle.fatal(new Error(sentinel));
  await h.lifecycle.capture(new Error(sentinel));
  assert.equal(h.binding,null);
  assert.equal(h.fatal.length,0);
  assert.equal(h.sent.length,0);
});

for (const [status, enabled, policy, action] of [
  ["available",true,"2","available"], ["not_offered",true,"2","disarm"],
  ["consent_required",true,"1","disarm"], ["consent_required",false,"2","disarm"],
  ["unknown",true,"2","pause"], [undefined,true,"2","pause"],
] as const) test(`permission ${status}/${enabled}/${policy} ${action}s without confusing unknown with withdrawal`, async () => {
  const h=harness(); await h.lifecycle.setRestoring(false); h.lifecycle.fatal(new Error(sentinel));
  const epoch=h.binding!.epochId, purges=h.purges;
  h.permission({diagnostics_status:status,enabled,policy_version:policy,diagnostics_available:status==="available"});
  await h.relaunch().setRestoring(false);
  if (action==="disarm") { assert.equal(h.binding,null); assert.equal(h.records.length,0); }
  if (action==="pause") { assert.equal(h.purges,purges); assert.equal(h.records.length,1); assert.equal(h.binding!.epochId,epoch);
    h.permission({diagnostics_status:"available",diagnostics_available:true}); await h.lifecycle.refresh(); assert.equal(h.records.length,0); assert.equal(h.sent.length,1);
  }
  if (action==="available") assert.equal(h.sent.length,1);
});
for (const [statusCode,errorCode,action] of [
  [400,1000,"drop"],[401,1000,"pause"],[403,1000,"pause"],[404,1000,"drop"],
  [407,1000,"pause"],[408,1000,"pause"],[409,1000,"drop"],[410,1000,"drop"],
  [413,1000,"drop"],[415,1000,"drop"],[422,1000,"drop"],
  [429,1000,"pause"],[503,1000,"pause"],[500,1000,"pause"],[0,0,"pause"],
  [403,13401,"pause"],[404,13400,"disarm"],
] as const) test(`HTTP ${statusCode}/${errorCode} ${action}s the persisted record`,async()=>{
  const h=harness(); await h.lifecycle.setRestoring(false); h.lifecycle.fatal(new Error(sentinel));
  const purges=h.purges, epoch=h.binding!.epochId;
  const native=h.native; let current=true;
  const send=createCrashRelay(async<T>(path: string)=>{
    if(path.endsWith("/analytics")) return {account_id:h.binding!.accountId,revision:h.binding!.revision,enabled:true,policy_version:"2",diagnostics_status:"available",diagnostics_available:true} as T;
    throw buildApiError({error: "refused", error_code: errorCode, message: "fixed"}, statusCode);
  },()=>current,()=>{native.disarm();current=false;},()=>0,()=>{current=false;native.pause();});
  const b=h.binding!; const r=revalidateNativeRecord(h.records[0]!)!.report;
  if(await send(r,b)) native.removeRecords([r.event_id]);
  if(action==="drop") {assert.equal(h.records.length,0);assert.equal(h.purges,purges);}
  else if(action==="disarm") {assert.equal(h.records.length,0);assert.equal(h.binding,null);}
  else {assert.equal(h.records.length,1);assert.equal(h.purges,purges);assert.equal(h.binding!.epochId,epoch);}
});

test("an invalid buffered record is dropped alone while valid records and binding survive", async () => {
  const h = harness();
  await h.lifecycle.setRestoring(false);
  h.lifecycle.fatal(new Error("first"));
  h.lifecycle.fatal(new Error("invalid"));
  h.lifecycle.fatal(new Error("third"));
  const invalid = JSON.parse(h.records[1]!);
  invalid.report.captured_at = new Date(Date.now() - 25 * 3600_000).toISOString();
  h.records[1] = JSON.stringify(invalid);
  const binding = { ...h.binding! }, purges = h.purges;
  await h.lifecycle.refresh();
  assert.deepEqual(h.binding, binding);
  assert.equal(h.purges, purges);
  assert.equal(h.records.length, 0);
  assert.equal(h.sent.length, 2);
  assert.ok(!JSON.stringify(h.sent).includes(invalid.report.event_id));
});

test("restore timeout without an account pauses then same-account restore resumes; sign-out and account change purge", async () => {
  const h = harness();
  await h.lifecycle.setRestoring(false);
  h.lifecycle.fatal(new Error("pending"));
  const binding = { ...h.binding! }, purges = h.purges;
  h.change({accountId: null, enabled: false});
  const next = h.relaunch();
  await next.setRestoring(false); // The auth splash timeout, before a bound account.
  assert.deepEqual(h.binding, binding);
  assert.equal(h.records.length, 1);
  assert.equal(h.purges, purges);
  assert.ok(h.pauses > 0);
  next.fatal(new Error("paused"));
  assert.equal(h.records.length, 1);
  h.change({accountId: binding.accountId, enabled: true});
  await next.refresh();
  assert.deepEqual(h.binding, binding);
  assert.equal(h.records.length, 0);
  assert.equal(h.sent.length, 1);
  next.fatal(new Error("sign-out"));
  next.disarm();
  assert.equal(h.binding, null);
  assert.equal(h.records.length, 0);
  await next.refresh();
  next.fatal(new Error("account-switch"));
  const other = randomUUID();
  h.change({accountId: other});
  h.permission({account_id: other, revision: randomUUID()});
  await next.refresh();
  assert.equal(h.records.length, 0);
  assert.equal(h.native.getBinding()!.accountId, other);
});

for (const [status, body, drop] of [
  [400, {error: "validation_error", error_code: 1000, message: "fixed"}, true],
  [403, "<html>captive portal</html>", false],
  [404, null, false],
  [400, {message: "proxy"}, false],
  [400, {error: "bad_request"}, false],
  [422, {error: "validation_error", error_code: "1000"}, false],
  [429, {error: "rate_limited", error_code: 1000}, false],
  [403, {error: "crash_report_consent_required", error_code: 13401}, false],
] as const) test(`persisted HTTP ${status} ${JSON.stringify(body)} drops only a coded NyxID refusal (${drop})`, async () => {
  const binding = bound();
  let paused = 0, disarmed = 0;
  const send = createCrashRelay(async<T>(path: string) => {
    if (path.endsWith("/analytics")) return {account_id: binding.accountId, revision: binding.revision,
      enabled: true, policy_version: "2", diagnostics_status: "available", diagnostics_available: true} as T;
    throw buildApiError(body, status);
  }, () => true, () => { disarmed++; }, () => 0, () => { paused++; });
  const record = revalidateNativeRecord(JSON.stringify({binding, report: report()}))!.report;
  assert.equal(await send(record, binding), drop);
  assert.equal(disarmed, 0);
  assert.equal(paused, drop ? 0 : 1);
});


test('R2 crash sanitization never evaluates retained local message getters', async () => {
  for (const error of [localizedError("Unable to verify account access. Please try again."), buildApiError(null, 503)]) {
    const h = harness(); await h.lifecycle.setRestoring(false);
    const getter = Object.getOwnPropertyDescriptor(error, 'message')!.get!;
    assert.ok(getter, 'production error retains a locale getter');
    let reads = 0;
    Object.defineProperty(error, 'message', {configurable:true,get:()=>{reads++;return getter.call(error);}});
    await h.lifecycle.capture(error);
    assert.equal(reads, 0, 'closed crash record must not evaluate localized display text');
    assert.equal(h.sent.length, 1, 'eligible fault still reports metadata');
    assert.ok(!JSON.stringify(h.sent).includes('Unable to verify'));
  }
});
