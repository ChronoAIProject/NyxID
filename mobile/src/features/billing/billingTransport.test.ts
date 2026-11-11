import assert from "node:assert/strict";
import test from "node:test";
import { ApiError } from "../../lib/api/ApiError";
import { createBillingApi } from "./billingTransport";
import { allowanceFixture, grantFixture, organizationsFixture, ORG_ADMIN, usageFixture, walletFixture } from "./billingFixtures";

test("every billing request is an explicit GET, with no body, provisioning, topup or checkout request", async () => {
  const requests: Array<{ path: string; options: unknown }> = [];
  const controller = new AbortController();
  const api = createBillingApi(async (path, options) => {
    requests.push({ path, options });
    assert.equal(options.method, "GET");
    assert.equal("body" in options, false);
    assert.equal(options.signal, controller.signal);
    if (path === "/billing/wallet/view") return walletFixture;
    if (path.startsWith("/billing/usage")) return { ...usageFixture, period: new URLSearchParams(path.split("?")[1]).get("period") };
    if (path.startsWith("/billing/grants")) return { grants: [grantFixture] };
    if (path.startsWith("/billing/allowances")) return { allowances: [allowanceFixture] };
    if (path === "/orgs") return organizationsFixture;
    throw new Error("Unexpected request");
  });
  await api.wallet(controller.signal);
  for (const period of ["24h", "7d", "30d"] as const) await api.usage(period, controller.signal);
  await api.grants(undefined, controller.signal);
  await api.allowances(undefined, controller.signal);
  await api.grants(ORG_ADMIN, controller.signal);
  await api.allowances(ORG_ADMIN, controller.signal);
  await api.organizations(controller.signal);
  assert.deepEqual(requests.map(({ path }) => path), [
    "/billing/wallet/view", "/billing/usage?period=24h", "/billing/usage?period=7d", "/billing/usage?period=30d",
    "/billing/grants", "/billing/allowances", `/billing/grants?owner_id=${ORG_ADMIN}`,
    `/billing/allowances?owner_id=${ORG_ADMIN}`, "/orgs",
  ]);
  assert.equal(requests.some(({ path }) => path === "/billing/wallet" || /topup|checkout/.test(path)), false);
  assert.deepEqual(Object.keys(api).sort(), ["allowances", "grants", "organizations", "usage", "wallet"]);
});

test("wallet absence is explicit null and an old server 404 never falls back to provisioning GET", async () => {
  const paths: string[] = [];
  const api = createBillingApi(async (path) => {
    paths.push(path);
    throw new ApiError({ statusCode: 404, errorKey: "not_found", errorCode: 1004, message: "Route not found" });
  });
  assert.deepEqual(await api.wallet(), { kind: "unsupported" });
  assert.deepEqual(paths, ["/billing/wallet/view"]);
  assert.deepEqual(await createBillingApi(async () => null).wallet(), { kind: "ready", data: null });
  assert.equal((await createBillingApi(async () => walletFixture).wallet()).kind, "ready");
});

test("invalid selectors are rejected before a request can be sent", async () => {
  let requests = 0;
  const api = createBillingApi(async () => { requests++; return {}; });
  assert.throws(() => api.grants("org&owner_id=another"));
  assert.throws(() => api.allowances("".padStart(100, "x")));
  await assert.rejects(api.usage("all" as "30d"));
  assert.equal(requests, 0);
});

test("unsupported and disabled responses are honest states, not fabricated empty balances", async () => {
  for (const error of [
    new ApiError({ statusCode: 404, errorKey: "not_found", errorCode: 1004, message: "Route not found" }),
    new ApiError({ statusCode: 402, errorKey: "billing_not_configured", errorCode: 11301, message: "Billing unavailable" }),
    new ApiError({ statusCode: 403, errorKey: "forbidden", errorCode: 1003, message: "Billing is not enabled for this account" }),
  ]) {
    const api = createBillingApi(async () => { throw error; });
    assert.deepEqual(await api.grants(), { kind: error.statusCode === 404 ? "unsupported" : "disabled" });
  }
  for (const payload of [{ enabled: false }, { billing_enabled: false }]) {
    assert.deepEqual(await createBillingApi(async () => payload).allowances(), { kind: "disabled" });
  }
  const denied = new ApiError({ statusCode: 403, errorKey: "forbidden", errorCode: 1003, message: "Org access denied" });
  await assert.rejects(createBillingApi(async () => { throw denied; }).grants(ORG_ADMIN), (error) => error === denied);
});

test("bad payloads and wrong usage periods fail visibly and a read may be retried", async () => {
  let requests = 0;
  const api = createBillingApi(async () => {
    requests++;
    return requests === 1 ? {} : usageFixture;
  });
  await assert.rejects(api.usage("30d"));
  assert.equal((await api.usage("30d")).kind, "ready");
  await assert.rejects(createBillingApi(async () => usageFixture).usage("7d"), /different usage period/);
  assert.equal(requests, 2);
});
