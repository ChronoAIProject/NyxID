import { beforeEach, expect, it } from "vitest";
import { act, renderHook } from "@testing-library/react";
import { useRemoteMachineContinuity } from "@/hooks/use-remote-machine-continuity";
import { useAuthStore } from "@/stores/auth-store";
import { remoteMachineFixture } from "@/test/remote-machine-fixture";
import {
  clearRemoteMachineIntent,
  getRemoteMachineIntent,
  readRemoteMachineIntent,
  acceptRemoteMachineIntent,
  remoteMachineAcceptedSnapshot,
  parseAcceptedRemoteMachineIntent,
  observeAcceptedRemoteMachineIntent,
} from "./remote-machine-intent";

const creation = {
  name: "my-remote-machine", capabilities: ["files", "shell"] as ("files" | "shell" | "browser" | "computer")[], specialist_id: null,
  profile_version: "remote-machine" as const, policy_version: "remote-machine-mvp" as const, data_policy_version: "remote-machine-data" as const,
  acknowledge_shared_environment: true, separation_choice: "automatic" as const, separation_policy_version: "remote-machine-separation-v1" as const, acknowledge_separation: true,
};
beforeEach(() => sessionStorage.clear());
const body = { action: "pause" as const, expected_revision: 4 };
it.each(["pause", "resume", "delete"] as const)("keeps the accepted %s identity through remount and current-ID handoff without leaking accounts", (action) => {
  const priorAuth = useAuthStore.getState();
  useAuthStore.setState({ user: { id: "owner" } as never });
  const old = { ...remoteMachineFixture, revision: 4, active_operation_id: "old-operation" };
  const intent = getRemoteMachineIntent("owner", `${old.node_id}:${action}`, { action, expected_revision: 4 });
  const view = renderHook(({ machine }) => useRemoteMachineContinuity(old.node_id, machine), { initialProps: { machine: old } });
  act(() => acceptRemoteMachineIntent("owner", old.node_id, action, intent.idempotencyKey, "new-operation"));
  expect(view.result.current).toMatchObject({ operationId: "new-operation", acceptedAction: action });
  view.unmount();
  const reopened = renderHook(({ machine }) => useRemoteMachineContinuity(old.node_id, machine), { initialProps: { machine: old } });
  expect(reopened.result.current.operationId).toBe("new-operation");
  reopened.rerender({ machine: { ...old, revision: 5, active_operation_id: "new-operation" } });
  expect(reopened.result.current.operationId).toBe("new-operation");
  // A list still holding the preceding read cannot undo the true handoff.
  reopened.rerender({ machine: old });
  expect(reopened.result.current.operationId).toBe("new-operation");
  reopened.rerender({ machine: { ...old, revision: 6, active_operation_id: "next-operation" } });
  expect(reopened.result.current.operationId).toBe("next-operation");
  reopened.rerender({ machine: { ...old, revision: 5, active_operation_id: "new-operation" } });
  expect(reopened.result.current.operationId).toBe("next-operation");
  expect(reopened.result.current.acceptedAction).toBeNull();
  reopened.unmount();
  const second = renderHook(({ machine }) => useRemoteMachineContinuity(old.node_id, machine), { initialProps: { machine: old } });
  expect(second.result.current.operationId).toBe("next-operation");
  second.rerender({ machine: { ...old, revision: 7, active_operation_id: "third-operation" } });
  second.rerender({ machine: { ...old, revision: 6, active_operation_id: "next-operation" } });
  expect(second.result.current.operationId).toBe("third-operation");
  expect(parseAcceptedRemoteMachineIntent(remoteMachineAcceptedSnapshot("owner", old.node_id))).toMatchObject({
    ...intent, operationId: "new-operation", action,
  });
  expect(remoteMachineAcceptedSnapshot("owner", "another-machine")).toBeNull();
  act(() => useAuthStore.setState({ user: { id: "other" } as never }));
  second.rerender({ machine: old });
  expect(second.result.current.operationId).toBe("old-operation");
  expect(second.result.current.acceptedAction).toBeNull();
  second.unmount();
  useAuthStore.setState(priorAuth);
});
it.each(["pause", "resume", "delete"] as const)("preserves the accepted %s response and exact reviewed request beyond the pending intent", (action) => {
  const intent = getRemoteMachineIntent("owner", `machine:${action}`, { action, expected_revision: 4 });
  acceptRemoteMachineIntent("owner", "machine", action, "wrong-key", "wrong-operation");
  expect(remoteMachineAcceptedSnapshot("owner", "machine")).toBeNull();
  acceptRemoteMachineIntent("owner", "machine", action, intent.idempotencyKey, "accepted-operation");
  const accepted = parseAcceptedRemoteMachineIntent(remoteMachineAcceptedSnapshot("owner", "machine"));
  expect(accepted).toEqual({ ...intent, action, operationId: "accepted-operation" });
  expect(readRemoteMachineIntent("owner", `machine:${action}`)).toBeNull();
  expect(remoteMachineAcceptedSnapshot("other", "machine")).toBeNull();
  expect(remoteMachineAcceptedSnapshot("owner", "other-machine")).toBeNull();
  observeAcceptedRemoteMachineIntent("owner", "machine", "old-operation", 3);
  expect(parseAcceptedRemoteMachineIntent(remoteMachineAcceptedSnapshot("owner", "machine"))).toEqual(accepted);
  observeAcceptedRemoteMachineIntent("owner", "machine", "accepted-operation", 5);
  observeAcceptedRemoteMachineIntent("owner", "machine", "accepted-operation", 4);
  expect(parseAcceptedRemoteMachineIntent(remoteMachineAcceptedSnapshot("owner", "machine"))?.observedRevision).toBe(5);
});
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
    ...creation,
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
    ...creation,
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

it("does not persist an incomplete create scope before dispatch", () => {
  expect(() => getRemoteMachineIntent("owner", "create", {
    name: "research", separation_choice: "automatic",
    separation_policy_version: "remote-machine-separation-v1", acknowledge_separation: true,
  })).toThrow(/complete/i);
  expect(sessionStorage.length).toBe(0);
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
    ...creation,
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
