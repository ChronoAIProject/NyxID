import { toast } from "sonner";
import { useAssistantPinsStore, PIN_STORAGE_KEY } from "@/stores/assistant-pins-store";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { QueryClient } from "@tanstack/react-query";
import { useAuthStore } from "@/stores/auth-store";
import { useAssistantDraftStore } from "@/stores/assistant-draft-store";
import { useAssistantContextStore } from "@/stores/assistant-context-store";
import {
  useAssistantWireLogStore,
  ASSISTANT_WIRE_LOG_STORAGE_KEY,
} from "@/stores/assistant-wire-log-store";
import { useCmaSessionStore } from "@/stores/cma-session-store";
import { apiClient, apiBlobClient, apiFetch } from "./api-client";
import { assistantHttp, assistantJson } from "./assistant/assistant-http";
import {
  bindAccountQueryClient,
  createAccountQueryScope,
} from "./account-query-cache";
import {
  AccountChangedError,
  isAccountAbort,
  releaseAccountResponse,
  captureAccountRequest,
  getAssistantIdentityRevision,
} from "./assistant/identity";
import type { User } from "@/types/api";

export const person = (id: string): User => ({
  id,
  email: `${id}@example.test`,
  display_name: id,
  avatar_url: null,
  email_verified: true,
  mfa_enabled: false,
  is_admin: false,
  is_active: true,
  created_at: "2026-11-01T00:00:00Z",
});
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => {
    resolve = done;
  });
  return { promise, resolve };
}
let client: QueryClient;
let stop: () => void;
beforeEach(() => {
  useAuthStore.getState().setUser(null);
  useAuthStore.setState({ isLoading: false });
  localStorage.clear();
  client = new QueryClient({ defaultOptions: { queries: { retry: false } } });
  stop = bindAccountQueryClient(client);
  globalThis.__nyxidAssistantHttpMock = undefined;
});
afterEach(() => {
  stop();
  client.clear();
  vi.restoreAllMocks();
  vi.unstubAllGlobals();
});

describe("account boundary", () => {
  it.each([
    ["A", "B"],
    ["B", "A"],
    ["A", null],
    [null, "B"],
  ])("clears queries before publishing %s -> %s", (from, to) => {
    useAuthStore.getState().setUser(from ? person(from) : null);
    client.setQueryData(["keys"], "private key metadata");
    const revision = getAssistantIdentityRevision();
    const generation = useAuthStore.getState().accountGeneration;
    const observed: unknown[] = [];
    const unsubscribe = useAuthStore.subscribe((state) => {
      if (state.user?.id === to || (to === null && !state.user))
        observed.push(client.getQueryData(["keys"]));
    });
    useAuthStore.getState().setUser(to ? person(to) : null);
    expect(observed).toEqual([undefined]);
    expect(getAssistantIdentityRevision()).toBe(revision + 1);
    expect(useAuthStore.getState().accountGeneration).toBe(
      generation + (from ? 1 : 0),
    );
    unsubscribe();
  });

  it("preserves the same account's cache, requests, generation and draft on setUser/checkAuth", async () => {
    useAuthStore.getState().setUser(person("A"));
    client.setQueryData(["keys"], "A metadata");
    useAssistantDraftStore
      .getState()
      .saveDraft("A", "draft", "unfinished A question");
    const revision = getAssistantIdentityRevision();
    const generation = useAuthStore.getState().accountGeneration;
    const request = captureAccountRequest();
    useAuthStore
      .getState()
      .setUser({ ...person("A"), display_name: "Updated" });
    vi.stubGlobal(
      "fetch",
      vi.fn().mockResolvedValue(Response.json(person("A"))),
    );
    await useAuthStore.getState().checkAuth({ ephemeral: true });
    expect(client.getQueryData(["keys"])).toBe("A metadata");
    expect(request.signal.aborted).toBe(false);
    expect(getAssistantIdentityRevision()).toBe(revision);
    expect(useAuthStore.getState().accountGeneration).toBe(generation);
    expect(useAssistantDraftStore.getState().getDraft("draft")).toBe(
      "unfinished A question",
    );
  });

  it("cancels a noncooperative query and removes both its reverted and late data", async () => {
    useAuthStore.getState().setUser(person("A"));
    client.setQueryData(["keys"], "A cached");
    const response = deferred<string>();
    const pending = client.fetchQuery({
      queryKey: ["keys"],
      queryFn: () => response.promise,
    });
    useAuthStore.getState().setUser(person("B"));
    await pending;
    response.resolve("late A keys");
    await Promise.resolve();
    expect(client.getQueryData(["keys"])).toBeUndefined();
  });

  it.each([false, true])(
    "drops old mutation success, error rollback and settled callbacks (error: %s)",
    async (fail) => {
      useAuthStore.getState().setUser(person("A"));
      const response = deferred<string>();
      const callbacks = {
        onSuccess: vi.fn(() => client.setQueryData(["keys"], "A secret")),
        onError: vi.fn(() => client.setQueryData(["keys"], "A rollback")),
        onSettled: vi.fn(),
      };
      const mutation = client.getMutationCache().build(client, {
        mutationFn: async () => {
          const data = await response.promise;
          if (fail) throw new Error("A failure");
          return data;
        },
        ...callbacks,
      });
      const pending = expect(mutation.execute(undefined)).rejects.toBeDefined();
      await vi.waitFor(() => expect(mutation.state.status).toBe("pending"));
      useAuthStore.getState().setUser(person("B"));
      response.resolve("A result");
      await pending;
      for (const callback of Object.values(callbacks))
        expect(callback).not.toHaveBeenCalled();
      expect(client.getMutationCache().getAll()).toEqual([]);
      expect(client.getQueryData(["keys"])).toBeUndefined();
    },
  );

  it("retires a callback's client even when onSuccess has already started awaiting", async () => {
    useAuthStore.getState().setUser(person("A"));
    const scope = createAccountQueryScope({});
    const old = scope.getClient();
    const pause = deferred<void>();
    const started = vi.fn();
    const mutation = old.getMutationCache().build(old, {
      mutationFn: async () => "A result",
      onSuccess: async (data) => {
        started();
        await pause.promise;
        old.setQueryData(["keys"], data);
      },
    });
    const pending = mutation.execute(undefined);
    await vi.waitFor(() => expect(started).toHaveBeenCalledOnce());
    useAuthStore.getState().setUser(person("B"));
    const current = scope.getClient();
    expect(current).not.toBe(old);
    current.setQueryData(["keys"], "B result");
    pause.resolve();
    await pending;
    expect(current.getQueryData(["keys"])).toBe("B result");
    const before = scope.getClient();
    useAuthStore
      .getState()
      .setUser({ ...person("B"), display_name: "Updated" });
    expect(scope.getClient()).toBe(before);
    scope.dispose();
  });

  it("rejects late A draft/context writes after B is published", () => {
    useAuthStore.getState().setUser(person("A"));
    useAssistantDraftStore.getState().saveDraft("A", "old", "A draft");
    useAuthStore.getState().setUser(person("B"));
    useAssistantDraftStore.getState().saveDraft("B", "new", "B draft");
    useAssistantDraftStore.getState().saveDraft("A", "old", "late A draft");
    useAssistantContextStore.getState().recordScreen("A", "/keys/A");
    expect(useAssistantDraftStore.getState().ownerUserId).toBe("B");
    expect(useAssistantDraftStore.getState().getDraft("old")).toBe("");
    expect(useAssistantDraftStore.getState().getDraft("new")).toBe("B draft");
    expect(useAssistantContextStore.getState().lastScreen).toBeNull();
  });

  it("clears legacy resource preference IDs on a real transition and retains ordinary layout preferences", () => {
    useAuthStore.getState().setUser(person("A"));
    localStorage.setItem("nyxid:last-picked-agent-key", "A-key");
    localStorage.setItem(
      "nyxid:desktop-collapsed:A-conversation:A-node:A-context",
      "1",
    );
    localStorage.setItem("nyxid:theme", "dark");
    useAuthStore
      .getState()
      .setUser({ ...person("A"), display_name: "Updated A" });
    expect(localStorage.getItem("nyxid:last-picked-agent-key")).toBe("A-key");
    expect(
      localStorage.getItem(
        "nyxid:desktop-collapsed:A-conversation:A-node:A-context",
      ),
    ).toBe("1");
    useAuthStore.getState().setUser(person("B"));
    expect(localStorage.getItem("nyxid:last-picked-agent-key")).toBeNull();
    expect(
      localStorage.getItem(
        "nyxid:desktop-collapsed:A-conversation:A-node:A-context",
      ),
    ).toBeNull();
    expect(localStorage.getItem("nyxid:theme")).toBe("dark");
  });

  it("an old logout completion cannot clear B", async () => {
    useAuthStore.getState().setUser(person("A"));
    const late = deferred<Response>();
    const fetch = vi.fn().mockReturnValue(late.promise);
    vi.stubGlobal("fetch", fetch);
    const pending = expect(
      useAuthStore.getState().logout(),
    ).rejects.toMatchObject({ name: "AbortError" });
    await vi.waitFor(() => expect(fetch).toHaveBeenCalledOnce());
    useAuthStore.getState().setUser(person("B"));
    client.setQueryData(["keys"], "B data");
    late.resolve(new Response(null, { status: 204 }));
    await pending;
    expect(useAuthStore.getState().user?.id).toBe("B");
    expect(client.getQueryData(["keys"])).toBe("B data");
  });

  it.each([
    "nyxid.assistant_drafts",
    "nyxid.assistant_context",
    "nyxid.cma-session",
    ASSISTANT_WIRE_LOG_STORAGE_KEY,
  ])("rejects previous-account rehydration: %s", async (key) => {
    useAuthStore.getState().setUser(person("A"));
    useAssistantDraftStore
      .getState()
      .saveDraft("A", "conv:old", "private draft");
    useAssistantContextStore.getState().recordScreen("A", "/keys/old");
    useCmaSessionStore.setState({ ownerId: "A", pinned: {}, minimised: {} });
    useAssistantWireLogStore.getState().setCaptureEnabled(true);
    const saved = localStorage.getItem(key)!;
    useAuthStore.getState().setUser(person("B"));
    localStorage.setItem(key, saved);
    await useAssistantDraftStore.persist.rehydrate();
    await useAssistantContextStore.persist.rehydrate();
    await useCmaSessionStore.persist.rehydrate();
    await useAssistantWireLogStore.persist.rehydrate();
    expect(useAssistantDraftStore.getState().getDraft("conv:old")).toBe("");
    expect(useAssistantContextStore.getState().lastScreen).toBeNull();
    expect(useCmaSessionStore.getState().ownerId).not.toBe("A");
    expect(useAssistantWireLogStore.getState().captureEnabled).toBe(false);
  });
});

describe("request epoch", () => {
  it.each([apiClient, assistantJson])(
    "drops successful JSON even if a response ignores abort",
    async (request) => {
      useAuthStore.getState().setUser(person("A"));
      const response = deferred<Response>();
      const fetch = vi.fn().mockReturnValue(response.promise);
      vi.stubGlobal("fetch", fetch);
      const pending = expect(request("/account-private")).rejects.toMatchObject(
        { name: "AbortError" },
      );
      await vi.waitFor(() => expect(fetch).toHaveBeenCalledOnce());
      useAuthStore.getState().setUser(person("B"));
      expect(fetch.mock.calls[0]![1].signal.aborted).toBe(true);
      response.resolve(Response.json({ text: "A private response" }));
      await pending;
    },
  );

  it.each([apiClient, assistantJson, apiBlobClient])(
    "fences body consumption after headers arrive",
    async (request) => {
      useAuthStore.getState().setUser(person("A"));
      const body = deferred<unknown>();
      const response = Response.json({});
      const json = vi
        .spyOn(response, "json")
        .mockImplementation(() => body.promise);
      const blob = vi.spyOn(response, "blob").mockImplementation(async () => {
        await body.promise;
        return new Blob(["A attachment"]);
      });
      vi.stubGlobal("fetch", vi.fn().mockResolvedValue(response));
      const pending = expect(request("/private-body")).rejects.toMatchObject({
        name: "AbortError",
      });
      await vi.waitFor(() =>
        expect(json.mock.calls.length + blob.mock.calls.length).toBe(1),
      );
      useAuthStore.getState().setUser(person("B"));
      body.resolve({ text: "A private body" });
      await pending;
    },
  );

  it.each([apiClient, assistantHttp])(
    "old 401 cannot sign B out",
    async (request) => {
      useAuthStore.getState().setUser(person("A"));
      const response = deferred<Response>();
      const fetch = vi.fn().mockReturnValue(response.promise);
      vi.stubGlobal("fetch", fetch);
      const pending = expect(request("/users/me")).rejects.toMatchObject({
        name: "AbortError",
      });
      await vi.waitFor(() => expect(fetch).toHaveBeenCalledOnce());
      useAuthStore.getState().setUser(person("B"));
      response.resolve(
        Response.json(
          { error: "unauthorized", error_code: 1001 },
          { status: 401 },
        ),
      );
      await pending;
      expect(useAuthStore.getState().user?.id).toBe("B");
    },
  );

  it("combines signals without AbortSignal.any and cleans listeners after success/cancellation", async () => {
    const originalAny = AbortSignal.any;
    Object.defineProperty(AbortSignal, "any", {
      configurable: true,
      value: undefined,
    });
    try {
      const caller = new AbortController();
      const add = vi.spyOn(caller.signal, "addEventListener");
      const remove = vi.spyOn(caller.signal, "removeEventListener");
      vi.stubGlobal(
        "fetch",
        vi.fn().mockResolvedValue(Response.json({ ok: true })),
      );
      await apiClient("/keys", { signal: caller.signal });
      for (const [, listener] of add.mock.calls)
        expect(
          remove.mock.calls.some(([, removed]) => removed === listener),
        ).toBe(true);
      const request = captureAccountRequest(caller.signal);
      caller.abort(new DOMException("Caller stopped", "AbortError"));
      expect(() => request.assertCurrent()).toThrow("Caller stopped");
      for (const [, listener] of add.mock.calls)
        expect(
          remove.mock.calls.some(([, removed]) => removed === listener),
        ).toBe(true);
      const next = captureAccountRequest(new AbortController().signal);
      useAuthStore.getState().setUser(person("A"));
      expect(next.signal.aborted).toBe(true);
    } finally {
      Object.defineProperty(AbortSignal, "any", {
        configurable: true,
        value: originalAny,
      });
    }
  });

  it.each(["consume", "cancel", "switch"])(
    "cleans fallback listeners when a response body ends through %s",
    async (end) => {
      const originalAny = AbortSignal.any;
      Object.defineProperty(AbortSignal, "any", {
        configurable: true,
        value: undefined,
      });
      try {
        useAuthStore.getState().setUser(person("A"));
        const caller = new AbortController();
        const add = vi.spyOn(caller.signal, "addEventListener");
        const remove = vi.spyOn(caller.signal, "removeEventListener");
        const cancelled = vi.fn();
        vi.stubGlobal("fetch", vi.fn(async (_url, init: RequestInit) => {
          if (end === "consume") return Response.json({ text: "A body" });
          return new Response(new ReadableStream({
            start(controller) { init.signal!.addEventListener("abort", () => { controller.error(init.signal!.reason); cancelled(); }, { once: true }); },
            cancel: cancelled,
          }));
        }));
        const response = await apiFetch("/private-stream", {
          signal: caller.signal,
        });
        if (end === "consume")
          expect(await response.json()).toEqual({ text: "A body" });
        else if (end === "cancel") {
          await response.body!.cancel();
          expect(cancelled).toHaveBeenCalledOnce();
        } else {
          const read = expect(response.text()).rejects.toMatchObject({
            name: "AbortError",
          });
          useAuthStore.getState().setUser(person("B"));
          await read;
          expect(cancelled).toHaveBeenCalledOnce();
        }
        releaseAccountResponse(response);
        expect(add).toHaveBeenCalled();
        for (const [, listener] of add.mock.calls)
          expect(
            remove.mock.calls.some(([, removed]) => removed === listener),
          ).toBe(true);
      } finally {
        Object.defineProperty(AbortSignal, "any", {
          configurable: true,
          value: originalAny,
        });
      }
    },
  );

  it("fences a buffered native clone with the caller scope after its original response has completed", async () => {
    const originalAny = AbortSignal.any;
    Object.defineProperty(AbortSignal, "any", {
      configurable: true,
      value: undefined,
    });
    try {
      const caller = new AbortController();
      const add = vi.spyOn(caller.signal, "addEventListener");
      const remove = vi.spyOn(caller.signal, "removeEventListener");
      const nativeFetch = globalThis.fetch;
      vi.stubGlobal("fetch", vi.fn((_url, init: RequestInit) => nativeFetch("data:application/json,%7B%22private%22%3A%22A%22%7D", init)));

      const response = await apiFetch("/private-body", {
        signal: caller.signal,
      });
      const account = captureAccountRequest(caller.signal);
      const clone = response.clone();
      expect(await response.json()).toEqual({ private: "A" });
      caller.abort(new DOMException("Caller cancelled", "AbortError"));
      await expect((async () => { const result = await clone.json(); account.assertCurrent(); return result; })()).rejects.toMatchObject({ name: "AbortError" });
      account.dispose();
      releaseAccountResponse(response);
      for (const [, listener] of add.mock.calls)
        expect(
          remove.mock.calls.some(([, removed]) => removed === listener),
        ).toBe(true);
    } finally {
      Object.defineProperty(AbortSignal, "any", {
        configurable: true,
        value: originalAny,
      });
    }
  });
});

it("distinguishes account aborts from caller cancellation and dismisses old toasts only on a real transition", () => {
  useAuthStore.getState().setUser(person("A"));
  const dismiss = vi.spyOn(toast, "dismiss");
  const caller = new AbortController();
  const own = captureAccountRequest(caller.signal);
  caller.abort(new DOMException("Caller cancelled", "AbortError"));
  try { own.assertCurrent(); } catch (error) { expect(isAccountAbort(error)).toBe(false); }
  expect(isAccountAbort(new AccountChangedError())).toBe(true);
  const account = captureAccountRequest();
  useAuthStore.getState().setUser({ ...person("A"), display_name: "updated" });
  expect(dismiss).not.toHaveBeenCalled();
  useAuthStore.getState().setUser(person("B"));
  expect(dismiss).toHaveBeenCalledOnce();
  try { account.assertCurrent(); } catch (error) { expect(isAccountAbort(error)).toBe(true); }
});

it("retries a failed pin-storage load on same-account confirmation and clears previous-owner pins on switch", () => {
  useAuthStore.getState().setUser(person("A"));
  localStorage.setItem(PIN_STORAGE_KEY, JSON.stringify({ version: 1, ownerUserId: "A", ids: ["A-conversation"] }));
  useAssistantPinsStore.setState({ ownerUserId: "A", error: "load", ids: [] });
  const original = localStorage.getItem.bind(localStorage);
  const read = vi.spyOn(localStorage, "getItem").mockImplementation((key) => { if (key === PIN_STORAGE_KEY) throw new Error("temporarily unavailable"); return original(key); });
  useAuthStore.getState().setUser(person("A"));
  expect(useAssistantPinsStore.getState().error).toBe("load");
  read.mockRestore();
  useAuthStore.getState().setUser(person("A"));
  expect(useAssistantPinsStore.getState().ids).toEqual(["A-conversation"]);
  expect(useAssistantPinsStore.getState().error).toBeNull();
  useAuthStore.getState().setUser(person("B"));
  expect(useAssistantPinsStore.getState().ids).toEqual([]);
  expect(localStorage.getItem(PIN_STORAGE_KEY)).toBeNull();
});
