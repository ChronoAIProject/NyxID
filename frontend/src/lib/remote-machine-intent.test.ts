import { beforeEach, expect, it } from "vitest";
import {
  clearRemoteMachineIntent,
  getRemoteMachineIntent,
  readRemoteMachineIntent,
} from "./remote-machine-intent";

beforeEach(() => sessionStorage.clear());
const body = { action: "pause" as const, expected_revision: 4 };
it("retains the original owner action and revision across retries and reload reads", () => {
  const first = getRemoteMachineIntent("owner", "machine:pause", body);
  const retry = getRemoteMachineIntent("owner", "machine:pause", {
    ...body,
    expected_revision: 5,
  });
  expect(retry).toEqual(first);
  expect(readRemoteMachineIntent("owner", "machine:pause")).toEqual(first);
  expect(retry.body.expected_revision).toBe(4);
});
it("separates account and action intents and only clears the matching acknowledgement", () => {
  const first = getRemoteMachineIntent("owner", "machine:pause", body);
  expect(
    getRemoteMachineIntent("other", "machine:pause", body).idempotencyKey,
  ).not.toBe(first.idempotencyKey);
  expect(
    getRemoteMachineIntent("owner", "machine:delete", {
      action: "delete",
      expected_revision: 4,
    }).idempotencyKey,
  ).not.toBe(first.idempotencyKey);
  clearRemoteMachineIntent("owner", "machine:pause", "wrong-key");
  expect(readRemoteMachineIntent("owner", "machine:pause")).toEqual(first);
  clearRemoteMachineIntent("owner", "machine:pause", first.idempotencyKey);
  expect(readRemoteMachineIntent("owner", "machine:pause")).toBeNull();
});
it("refuses fields which could persist secrets or work content", () => {
  expect(() =>
    getRemoteMachineIntent("owner", "machine:pause", {
      ...body,
      command: "private command",
    } as typeof body),
  ).toThrow();
  expect(sessionStorage.length).toBe(0);
});
it("does not dispatch a replacement intent when stored data is corrupt", () => {
  sessionStorage.setItem(
    "nyxid:remote-machine-intent:owner:machine%3Apause",
    "invalid",
  );
  expect(() =>
    getRemoteMachineIntent("owner", "machine:pause", body),
  ).toThrow();
});
it("retains the accepted quote and balance terms after an unknown paid-create result", () => {
  const paid = {
    free_slot: false,
    policy_version: "remote-machine-mvp" as const,
    data_policy_version: "remote-machine-data" as const,
    acknowledge_shared_environment: true,
    quote_id: "quote-one",
    accept_account_balance: true,
  };
  const first = getRemoteMachineIntent("owner", "create", paid);
  const retry = getRemoteMachineIntent("owner", "create", {
    ...paid,
    quote_id: "quote-two",
  });
  expect(retry).toEqual(first);
  expect(readRemoteMachineIntent("owner", "create")?.body.quote_id).toBe(
    "quote-one",
  );
  expect(
    readRemoteMachineIntent("owner", "create")?.body.accept_account_balance,
  ).toBe(true);
});
it("retains both the original revision and quote for a paid resume retry", () => {
  const paid = {
    action: "resume" as const,
    expected_revision: 4,
    quote_id: "quote-one",
    accept_account_balance: true,
  };
  const first = getRemoteMachineIntent("owner", "machine:resume", paid);
  expect(
    getRemoteMachineIntent("owner", "machine:resume", {
      ...paid,
      expected_revision: 5,
      quote_id: "quote-two",
    }),
  ).toEqual(first);
});
it("rejects partial paid acceptance, accidental free conversion and secret fields", () => {
  for (const paid of [
    { free_slot: false },
    { free_slot: false, quote_id: "quote-one" },
    { free_slot: true, quote_id: "quote-one", accept_account_balance: true },
    { action: "pause", quote_id: "quote-one", accept_account_balance: true },
    {
      free_slot: false,
      quote_id: "quote-one",
      accept_account_balance: true,
      token: "secret",
    },
  ])
    expect(() =>
      getRemoteMachineIntent(
        "owner",
        "create",
        paid as Parameters<typeof getRemoteMachineIntent>[2],
      ),
    ).toThrow();
  expect(sessionStorage.length).toBe(0);
});

it("persists hosted configuration and retries its original body and key", () => {
  const body = {
    free_slot: true,
    name: "research",
    capabilities: ["browser" as const, "computer" as const],
    specialist_id: "specialist-1",
    policy_version: "remote-machine-mvp" as const,
    data_policy_version: "remote-machine-data" as const,
    acknowledge_shared_environment: true,
  };
  const first = getRemoteMachineIntent("owner", "create", body);
  expect(readRemoteMachineIntent("owner", "create")).toEqual(first);
  expect(first.body).toEqual(body);
  expect(
    getRemoteMachineIntent("owner", "create", {
      ...body,
      name: "replacement",
      capabilities: ["shell"],
      specialist_id: null,
    }),
  ).toEqual(first);
});
it("rejects invalid hosted persisted configuration without changing action intents", () => {
  for (const patch of [
    { name: "Bad Name" },
    { capabilities: [] },
    { capabilities: ["computer" as const] },
  ])
    expect(() =>
      getRemoteMachineIntent("owner", "create", { free_slot: true, ...patch }),
    ).toThrow();
  expect(getRemoteMachineIntent("owner", "machine:pause", body).body).toEqual(
    body,
  );
});

it.each(["create", "machine:resume", "machine:delete"])("rejects obsolete stored %s consent without migration", (scope) => {
  const key = `nyxid:remote-machine-intent:owner:${encodeURIComponent(scope)}`;
  for (const patch of [{ policy_version: "remote-machine-mvp-v1" }, { policy_version: "remote-machine-mvp-v2" }, { data_policy_version: "remote-machine-data-v1" }]) {
    const raw = JSON.stringify({ idempotencyKey: crypto.randomUUID(), body: { ...patch } });
    sessionStorage.setItem(key, raw);
    expect(() => readRemoteMachineIntent("owner", scope)).toThrow(/reset/);
    expect(sessionStorage.getItem(key)).toBe(raw);
    clearRemoteMachineIntent("owner", scope);
    expect(readRemoteMachineIntent("owner", scope)).toBeNull();
  }
});


it("retains the accepted profile with the original quote and rejects replacement profile identities", () => {
  const paid = {
    free_slot: false,
    profile_version: "remote-machine" as const,
    quote_id: "quote-one",
    accept_account_balance: true,
  };
  const first = getRemoteMachineIntent("owner", "create", paid);
  expect(readRemoteMachineIntent("owner", "create")?.body.profile_version).toBe(
    "remote-machine",
  );
  expect(() =>
    getRemoteMachineIntent("owner", "create", {
      ...paid,
      profile_version: "foreign-profile",
    } as unknown as typeof paid),
  ).toThrow();
  expect(readRemoteMachineIntent("owner", "create")).toEqual(first);
});
