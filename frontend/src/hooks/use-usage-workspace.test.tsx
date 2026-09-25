import { act, renderHook } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { api } from "@/lib/api-client";
import { newView } from "@/lib/usage-analytics";
import type {
  WorkspaceConfig,
  WorkspaceResponse,
} from "@/schemas/usage-analytics";
import { useAutosavedWorkspace } from "./use-usage-workspace";
vi.mock("@/lib/api-client", () => ({ api: { get: vi.fn(), put: vi.fn() } }));
const initial = (): WorkspaceResponse => ({
  revision: 1,
  config: { version: 1, draft: newView(), saved_views: [] },
});
beforeEach(() => {
  vi.useFakeTimers();
  localStorage.clear();
  sessionStorage.clear();
  vi.resetAllMocks();
});
afterEach(() => {
  vi.useRealTimers();
});
async function tick() {
  await act(async () => {
    await vi.advanceTimersByTimeAsync(650);
  });
}
it("serializes autosaves and keeps edits made while a request is in flight", async () => {
  const stored = initial();
  let resolveFirst!: (response: WorkspaceResponse) => void;
  vi.mocked(api.put)
    .mockImplementationOnce(
      () =>
        new Promise((resolve) => {
          resolveFirst = resolve;
        }),
    )
    .mockImplementationOnce(async (_path, body) => ({
      revision: 3,
      config: (body as { config: WorkspaceConfig }).config,
    }));
  const { result } = renderHook(() =>
    useAutosavedWorkspace(stored, "alice", true),
  );
  const first = {
    ...stored.config!,
    draft: { ...stored.config!.draft, name: "First edit" },
  };
  act(() => result.current.setConfig(first));
  await tick();
  const second = { ...first, draft: { ...first.draft, name: "Second edit" } };
  act(() => result.current.setConfig(second));
  await tick();
  expect(api.put).toHaveBeenCalledTimes(1);
  await act(async () => resolveFirst({ revision: 2, config: first }));
  await tick();
  expect(api.put).toHaveBeenLastCalledWith("/admin/usage/workspace", {
    revision: 2,
    config: second,
  });
  expect(result.current.config.draft.name).toBe("Second edit");
  expect(result.current.dirty).toBe(false);
});
it("recovers an unsent draft without leaking it to another user", async () => {
  const stored = initial();
  const first = renderHook(() => useAutosavedWorkspace(stored, "alice", true));
  act(() =>
    first.result.current.setConfig({
      ...stored.config!,
      draft: { ...stored.config!.draft, name: "Recovered draft" },
    }),
  );
  first.unmount();
  const recovered = renderHook(() =>
    useAutosavedWorkspace(stored, "alice", true),
  );
  expect(recovered.result.current.config.draft.name).toBe("Recovered draft");
  expect(recovered.result.current.dirty).toBe(true);
  recovered.unmount();
  const bob = renderHook(() => useAutosavedWorkspace(stored, "bob", true));
  expect(bob.result.current.config.draft.name).toBe("Operations");
});
it("keeps another tab's recovery draft when this tab finishes saving", async () => {
  const stored = initial();
  let finish!: (response: WorkspaceResponse) => void;
  vi.mocked(api.put).mockImplementationOnce(
    () =>
      new Promise((resolve) => {
        finish = resolve;
      }),
  );
  const { result } = renderHook(() =>
    useAutosavedWorkspace(stored, "alice", true),
  );
  const edited = {
    ...stored.config!,
    draft: { ...stored.config!.draft, name: "This tab" },
  };
  act(() => result.current.setConfig(edited));
  await tick();
  const key = "nyxid:usage-workspace:v1:alice";
  const otherTab = JSON.stringify({
    revision: 1,
    config: { ...edited, draft: { ...edited.draft, name: "Another tab" } },
    writer: "another-tab",
  });
  localStorage.setItem(key, otherTab);
  await act(async () => finish({ revision: 2, config: edited }));
  expect(localStorage.getItem(key)).toBe(otherTab);
  expect(sessionStorage.getItem(key)).toBeNull();
  expect(result.current.dirty).toBe(false);
});
it("recovers this tab's draft before another tab's more recent local backup", () => {
  const stored = initial();
  const hook = renderHook(() => useAutosavedWorkspace(stored, "alice", true));
  const edited = {
    ...stored.config!,
    draft: { ...stored.config!.draft, name: "This tab" },
  };
  act(() => hook.result.current.setConfig(edited));
  hook.unmount();
  localStorage.setItem(
    "nyxid:usage-workspace:v1:alice",
    JSON.stringify({
      revision: 1,
      config: { ...edited, draft: { ...edited.draft, name: "Another tab" } },
    }),
  );
  const recovered = renderHook(() =>
    useAutosavedWorkspace(stored, "alice", true),
  );
  expect(recovered.result.current.config.draft.name).toBe("This tab");
});
it("pauses conflicts and only uses a fresh revision after explicit recovery", async () => {
  const stored = initial();
  vi.mocked(api.put).mockRejectedValueOnce(new Error("Changed in another tab"));
  const { result } = renderHook(() =>
    useAutosavedWorkspace(stored, "alice", true),
  );
  const edited = {
    ...stored.config!,
    draft: { ...stored.config!.draft, name: "My changes" },
  };
  act(() => result.current.setConfig(edited));
  await tick();
  expect(result.current.error).toContain("another tab");
  await tick();
  expect(api.put).toHaveBeenCalledTimes(1);
  vi.mocked(api.get).mockResolvedValue({ revision: 7, config: stored.config });
  vi.mocked(api.put).mockResolvedValue({ revision: 8, config: edited });
  await act(async () => result.current.reload(true));
  await tick();
  expect(api.put).toHaveBeenLastCalledWith("/admin/usage/workspace", {
    revision: 7,
    config: edited,
  });
});
it("does not write for operators or incomplete custom ranges", async () => {
  const stored = initial();
  const operator = renderHook(() =>
    useAutosavedWorkspace(stored, "operator", false),
  );
  act(() =>
    operator.result.current.setConfig({
      ...stored.config!,
      draft: { ...stored.config!.draft, name: "Local exploration" },
    }),
  );
  await tick();
  expect(api.put).not.toHaveBeenCalled();
  operator.unmount();
  const editor = renderHook(() => useAutosavedWorkspace(stored, "alice", true));
  act(() =>
    editor.result.current.setConfig({
      ...stored.config!,
      draft: {
        ...stored.config!.draft,
        filters: {
          ...stored.config!.draft.filters,
          period: "custom",
          from: null,
          to: null,
        },
      },
    }),
  );
  await tick();
  expect(api.put).not.toHaveBeenCalled();
  expect(editor.result.current.valid).toBe(false);
});
it("saves the first workspace and reports persistence failures", async () => {
  vi.mocked(api.put).mockRejectedValue(new Error("Offline"));
  const { result } = renderHook(() =>
    useAutosavedWorkspace({ revision: 0, config: null }, "alice", true),
  );
  await tick();
  expect(api.put).toHaveBeenCalledTimes(1);
  expect(result.current.error).toBe("Offline");
  expect(result.current.dirty).toBe(true);
});

it.each([1, 0])(
  "shows the saved view while preserving an incomplete recovered draft at revision %s",
  async (revision) => {
    const stored = initial();
    const incomplete = structuredClone(stored.config!);
    incomplete.draft.filters = {
      ...incomplete.draft.filters,
      period: "custom",
      from: null,
      to: null,
      services: ["llm-cohere", "aws-cost-explorer"],
    };
    const raw = JSON.stringify({ revision, config: incomplete });
    const key = "nyxid:usage-workspace:v1:alice";
    sessionStorage.setItem(key, raw);
    localStorage.setItem(key, raw);
    const { result, unmount } = renderHook(() =>
      useAutosavedWorkspace(stored, "alice", true),
    );
    expect(result.current.config).toEqual(stored.config);
    expect(result.current.pendingRecovery).toEqual(incomplete);
    expect(result.current.valid).toBe(true);
    expect(result.current.error).toBeNull();
    await tick();
    expect(api.put).not.toHaveBeenCalled();
    expect(sessionStorage.getItem(key)).toBe(raw);
    expect(localStorage.getItem(key)).toBe(raw);
    unmount();
    const reopened = renderHook(() =>
      useAutosavedWorkspace(stored, "alice", true),
    );
    expect(reopened.result.current.config).toEqual(stored.config);
    expect(reopened.result.current.pendingRecovery).toEqual(incomplete);
    vi.mocked(api.get).mockResolvedValue(stored);
    await act(async () => reopened.result.current.reload());
    expect(reopened.result.current.pendingRecovery).toBeNull();
    expect(sessionStorage.getItem(key)).toBeNull();
    expect(localStorage.getItem(key)).toBeNull();
    await tick();
    expect(api.put).not.toHaveBeenCalled();
  },
);

it("restores a stale draft only on request and writes with the latest server revision", async () => {
  const stored = initial();
  const recovered = structuredClone(stored.config!);
  recovered.draft.name = "My recovered view";
  recovered.draft.filters.services = ["llm-cohere"];
  const key = "nyxid:usage-workspace:v1:alice";
  localStorage.setItem(key, JSON.stringify({ revision: 0, config: recovered }));
  const { result } = renderHook(() =>
    useAutosavedWorkspace(stored, "alice", true),
  );
  expect(result.current.config).toEqual(stored.config);
  expect(result.current.pendingRecovery).toEqual(recovered);
  await tick();
  expect(api.put).not.toHaveBeenCalled();
  vi.mocked(api.get).mockResolvedValue({ revision: 8, config: stored.config });
  vi.mocked(api.put).mockResolvedValue({ revision: 9, config: recovered });
  await act(async () => result.current.reload(true));
  expect(result.current.config).toEqual(recovered);
  expect(result.current.pendingRecovery).toBeNull();
  await tick();
  expect(api.put).toHaveBeenCalledWith("/admin/usage/workspace", {
    revision: 8,
    config: recovered,
  });
  expect(result.current.dirty).toBe(false);
});

it("keeps another tab's backup when dismissing the recovered draft", async () => {
  const stored = initial();
  const key = "nyxid:usage-workspace:v1:alice";
  const old = JSON.stringify({
    revision: 0,
    config: {
      ...stored.config!,
      draft: { ...stored.config!.draft, name: "Old" },
    },
  });
  sessionStorage.setItem(key, old);
  localStorage.setItem(key, old);
  const { result } = renderHook(() =>
    useAutosavedWorkspace(stored, "alice", true),
  );
  const other = JSON.stringify({
    revision: 1,
    config: {
      ...stored.config!,
      draft: { ...stored.config!.draft, name: "Another tab" },
    },
  });
  localStorage.setItem(key, other);
  vi.mocked(api.get).mockResolvedValue(stored);
  await act(async () => result.current.reload());
  expect(result.current.config).toEqual(stored.config);
  expect(sessionStorage.getItem(key)).toBeNull();
  expect(localStorage.getItem(key)).toBe(other);
});
