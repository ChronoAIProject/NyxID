import { afterEach, expect, it, vi } from "vitest";
import { apiClient, apiFetch } from "./api-client";
import { assistantHttp } from "./assistant/assistant-http";
import {
  captureAccountRequest,
  AccountChangedError,
} from "./assistant/identity";
import { useAuthStore } from "@/stores/auth-store";
import { installAccountSessionSync } from "./account-session-sync";
import type { User } from "@/types/api";

let stop: (() => void) | undefined;
afterEach(() => {
  stop?.();
  vi.unstubAllGlobals();
  delete globalThis.__nyxidAssistantHttpMock;
});
function evidence() {
  window.dispatchEvent(
    new StorageEvent("storage", {
      key: "nyxid:account-session",
      newValue: JSON.stringify({ userId: "B", nonce: crypto.randomUUID() }),
    }),
  );
}
it.each([apiClient, apiFetch, assistantHttp])(
  "refuses pre-evidence writes while GET remains usable (%s)",
  async (send) => {
    useAuthStore.getState().setUser({ id: "A" } as User);
    vi.stubGlobal(
      "BroadcastChannel",
      class {
        close() {}
        postMessage() {}
      },
    );
    const fetch = vi.fn((url: string) =>
      String(url).endsWith("/users/me")
        ? new Promise(() => {})
        : Promise.resolve(Response.json({ ok: true })),
    );
    vi.stubGlobal("fetch", fetch);
    stop = installAccountSessionSync();
    const scope = captureAccountRequest();
    evidence();
    await expect(
      send("/fixture-effect", { method: "POST", accountRequest: scope }),
    ).rejects.toBeInstanceOf(AccountChangedError);
    expect(
      fetch.mock.calls.filter(([url]) =>
        String(url).endsWith("/fixture-effect"),
      ),
    ).toHaveLength(0);
    await expect(
      send("/fixture-read", { accountRequest: scope }),
    ).resolves.toBeDefined();
  },
);
it("checks again after the assistant mock await before native dispatch", async () => {
  useAuthStore.getState().setUser({ id: "A" } as User);
  vi.stubGlobal(
    "BroadcastChannel",
    class {
      close() {}
      postMessage() {}
    },
  );
  const fetch = vi.fn((url: string) => {
    void url;
    return new Promise(() => {});
  });
  vi.stubGlobal("fetch", fetch);
  stop = installAccountSessionSync();
  let resume!: (value: undefined) => void;
  globalThis.__nyxidAssistantHttpMock = () =>
    new Promise((done) => {
      resume = done;
    });
  const write = assistantHttp("/fixture-effect", { method: "POST" });
  evidence();
  resume(undefined);
  await expect(write).rejects.toBeInstanceOf(AccountChangedError);
  expect(
    fetch.mock.calls.filter(([url]) => String(url).endsWith("/fixture-effect")),
  ).toHaveLength(0);
});
it("retains a write response already dispatched before a stale hint confirms the same identity", async () => {
  useAuthStore.getState().setUser({ id: "A" } as User);
  vi.stubGlobal(
    "BroadcastChannel",
    class {
      close() {}
      postMessage() {}
    },
  );
  let respond!: (response: Response) => void;
  const fetch = vi.fn((url: string) =>
    String(url).endsWith("/users/me")
      ? Promise.resolve(Response.json({ id: "A" }))
      : new Promise<Response>((done) => {
          respond = done;
        }),
  );
  vi.stubGlobal("fetch", fetch);
  stop = installAccountSessionSync();
  const result = apiClient("/fixture-effect", { method: "POST" });
  await vi.waitFor(() => expect(respond).toBeDefined());
  evidence();
  respond(Response.json({ saved: true }));
  await expect(result).resolves.toEqual({ saved: true });
});

it("revokes a queued Query mutation before onMutate releases, without discarding an issued write", async () => {
  const { QueryClient, MutationObserver } =
    await import("@tanstack/react-query");
  const { bindAccountQueryClient } = await import("./account-query-cache");
  const { invalidateAccountEvidence, settleAccountEvidence } =
    await import("./assistant/identity");
  const client = new QueryClient({
    defaultOptions: { mutations: { retry: false } },
  });
  const stopBinding = bindAccountQueryClient(client);
  let release!: () => void;
  const held = new Promise<void>((done) => {
    release = done;
  });
  const dispatch = vi.fn().mockResolvedValue("saved");
  const onError = vi.fn();
  const observer = new MutationObserver(client, {
    onMutate: () => held,
    mutationFn: dispatch,
    onError,
  });
  try {
    const result = observer.mutate(undefined);
    const rejected = expect(result).rejects.toBeInstanceOf(AccountChangedError);
    invalidateAccountEvidence();
    settleAccountEvidence();
    release();
    await rejected;
    expect(dispatch).not.toHaveBeenCalled();
    expect(onError).not.toHaveBeenCalled();
    // A fresh user action after verification still succeeds.
    await expect(observer.mutate(undefined)).resolves.toBe("saved");
    expect(dispatch).toHaveBeenCalledOnce();
    let reply!: (value: string) => void;
    const issued = new MutationObserver(client, {
      mutationFn: () =>
        new Promise<string>((done) => {
          reply = done;
        }),
    });
    const pending = issued.mutate();
    await vi.waitFor(() => expect(reply).toBeDefined());
    invalidateAccountEvidence();
    settleAccountEvidence();
    reply("already issued");
    await expect(pending).resolves.toBe("already issued");
  } finally {
    settleAccountEvidence();
    stopBinding();
    client.clear();
  }
});
