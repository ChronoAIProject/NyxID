import test from "node:test";
import assert from "node:assert/strict";
import path from "node:path";
import {QueryClient, QueryObserver, MutationObserver} from "@tanstack/query-core";
import {hookHarness, deferred} from "../assistant/assistantHookTestHarness";
import {ApiError} from "../../lib/api/ApiError";
import * as authEpoch from "../../lib/auth/authEpoch";
import * as authAccount from "../../lib/auth/authAccount";
import type {QuickAccessState, QuickAccessItem} from "../../lib/api/quickAccessSchema";

const state = (...ids: string[]): QuickAccessState => ({customized: true,
  items: ids.map(id => ({kind: "destination", id} as QuickAccessItem))});
const ids = (value: QuickAccessState | undefined) => value?.items.map(item => item.id);

for (const answer of ["503", "success"] as const) {
  for (const replacement of [null, "other-account", "same-account"] as const) {
    test(`R1-D-c ${answer}: ${replacement ? `${replacement} replacement never receives the old result or recreated metadata` : "failed sign-out releases the old write and reconciles the surviving account"}`, async () => {
      const disk = new Map<string, string>();
      let failTombstone = false;
      const storeHarness = hookHarness({
        "expo-secure-store": {
          getItemAsync: async (key: string) => disk.get(key) ?? null,
          setItemAsync: async (key: string, value: string) => {
            if (failTombstone && JSON.parse(value).session === null)
              throw Error("tombstone write failed");
            disk.set(key, value);
          },
          deleteItemAsync: async (key: string) => {disk.delete(key);},
        },
        "../analyticsConsentState": {analyticsConsentState: {bindAccount: () => {}}},
      });
      const sessionStore = storeHarness.load(path.resolve("src/lib/auth/sessionStore.ts")) as typeof import("../../lib/auth/sessionStore");
      const token = "e30." + Buffer.from(JSON.stringify({sub: "self"})).toString("base64url") + ".fixture";
      await sessionStore.persistAuthSession({accessToken: token, refreshToken: "fixture-refresh"});
      authAccount.setAuthAccountId("self");
      const epoch = authEpoch.getAuthEpoch(), generation = sessionStore.authSessionGeneration();
      const client = new QueryClient(), key = ["account", "quick-access", epoch];
      let server = state("plugins"), reads = 0;
      client.setQueryData(key, server);
      const sent = deferred<void>(), reply = deferred<void>();
      const queries = new Map<string, QueryObserver<QuickAccessState>>();
      let mutation: MutationObserver<any, any, any> | undefined;
      let settled: Promise<boolean> | undefined;
      const hook = hookHarness<ReturnType<typeof import("./useQuickAccess").useQuickAccess>>({
        "react-native": {AppState: {addEventListener: () => ({remove() {}})}},
        "../../lib/auth/sessionStore": sessionStore,
        "../../lib/api/quickAccessApi": {quickAccessApi: {
          get: async () => {reads++; return server;},
          save: async (items: QuickAccessItem[], guard: () => void) => {
            guard(); sent.resolve(); await reply.promise;
            server = {items, customized: true}; return server;
          },
        }},
        "@tanstack/react-query": {
          useQueryClient: () => client, useIsMutating: (filters: any) => client.isMutating(filters),
          useQuery: (options: any) => {
            const k = JSON.stringify(options.queryKey);
            if (!queries.has(k)) {
              const q = new QueryObserver<QuickAccessState>(client, options);
              q.subscribe(() => {}); queries.set(k, q);
            }
            return queries.get(k)!.getCurrentResult();
          },
          useMutation: (options: any) => {
            if (!mutation) {
              mutation = new MutationObserver(client, options); mutation.subscribe(() => {});
            }
            return {...mutation.getCurrentResult(), mutate: (change: any, cb: any) => {
              settled = mutation!.mutate(change, cb).then(() => true, () => false);
            }, reset: () => mutation!.reset()};
          },
        },
      });
      const mod = hook.load(path.resolve("src/features/quickAccess/useQuickAccess.ts")) as typeof import("./useQuickAccess");
      try {
        hook.mount(() => mod.useQuickAccess());
        hook.value().save(state("plugins", "machines").items);
        await sent.promise;
        failTombstone = true;
        await assert.rejects(sessionStore.clearStoredAuthSession(), /tombstone write failed/);
        assert.equal(authEpoch.getAuthEpoch(), epoch);
        assert.equal(authAccount.getAuthAccountId(), "self");
        assert.equal(sessionStore.authSessionGeneration(), generation + 1);
        assert.equal((await sessionStore.loadStoredAuthSession())!.refreshToken, "fixture-refresh");
        if (replacement) {
          queries.forEach(q => q.destroy()); client.clear();
          if (replacement === "other-account") {authAccount.setAuthAccountId("replacement"); authEpoch.advanceAuthEpoch();}
          client.setQueryData(["account", "quick-access", authEpoch.getAuthEpoch()], state("activity"));
        }
        if (answer === "503") reply.reject(new ApiError({statusCode: 503, errorCode: 1000, errorKey: "unavailable", message: "unavailable"}));
        else reply.resolve();
        assert.equal(await settled, answer === "success");
        assert.equal(client.isMutating(), 0);
        if (replacement) {
          assert.equal(reads, 0, "the retired account never refetches under the replacement");
          if (replacement === "other-account")
            assert.equal(client.getQueryData(key), undefined, "no old list was republished");
          assert.equal(client.getQueryData(["account", "quick-access-write", epoch]), undefined);
          assert.equal(client.getQueryData(["account", "quick-access-conflict", epoch]), undefined);
          assert.deepEqual(ids(client.getQueryData(["account", "quick-access", authEpoch.getAuthEpoch()])), ["activity"]);
          assert.equal(client.getQueryCache().getAll().length, 1, "no old metadata query was recreated");
        } else {
          // Mutation settlement must initiate reconciliation even when its old
          // generation no longer has permission to publish its response.
          await new Promise(resolve => setImmediate(resolve));
          assert.ok(reads > 0, "same-account revoked authority needs a server reconciliation");
          server = {...server, items: [...server.items, ...state("settings").items]};
          const before = reads, observer = queries.get(JSON.stringify(key))!;
          await observer.refetch(); await observer.refetch();
          assert.ok(reads > before, "a settled write must not suppress manual refreshes");
          assert.deepEqual(ids(client.getQueryData(key)), ids(server));
          assert.equal(client.getQueryData<{pending: number}>(["account", "quick-access-write", epoch])?.pending, 0);
          if (answer === "503") assert.deepEqual(ids(client.getQueryData(key)), ["plugins", "settings"], "the unsaved Machines edit is removed");
        }
      } finally {
        hook.unmount(); queries.forEach(q => q.destroy()); client.clear(); authAccount.setAuthAccountId(null);
      }
    });
  }
}
