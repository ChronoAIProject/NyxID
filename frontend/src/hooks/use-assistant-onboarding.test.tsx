import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, cleanup, renderHook, waitFor } from "@testing-library/react";
import type { PropsWithChildren } from "react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { nyxBotApi } from "@/lib/assistant/nyxbot-api";
import { nyxBotQueryKeys } from "@/hooks/use-nyxbot-agents";
import { useAuthStore } from "@/stores/auth-store";
import type { User } from "@/types/api";
import type { AssistantOnboarding } from "@/schemas/assistant-onboarding";
import { useHideSuggestions, useShowSuggestions } from "./use-assistant-onboarding";

vi.mock("@/lib/assistant/nyxbot-api", () => ({
  nyxBotApi: { hideSuggestions: vi.fn(), showSuggestions: vi.fn() },
}));

afterEach(cleanup);
beforeEach(() => {
  vi.resetAllMocks();
  useAuthStore.setState({ user: { id: "owner" } as User });
});

const state: AssistantOnboarding = {
  show_intro: false, suggestions_hidden: true, intro_version: 2,
  profile: { outcome: "completed", topics: ["inbox"], work_style: "solo", intro_version: 1, updated_at: "2026-11-12T00:00:00Z" },
  topics: [], work_styles: [], recommendations: [],
};
const key = nyxBotQueryKeys.onboarding("owner");

function setup(initial = state) {
  const client = new QueryClient({ defaultOptions: { mutations: { retry: false }, queries: { retry: false } } });
  client.setQueryData(key, initial);
  const wrapper = ({ children }: PropsWithChildren) => <QueryClientProvider client={client}>{children}</QueryClientProvider>;
  return { client, ...renderHook(() => ({ hide: useHideSuggestions(), show: useShowSuggestions() }), { wrapper }) };
}

function deferred<T>() {
  let resolve!: (value: T) => void;
  let reject!: (reason: Error) => void;
  const promise = new Promise<T>((yes, no) => { resolve = yes; reject = no; });
  return { promise, resolve, reject };
}

it("optimistically shows suggestions and adopts the refreshed server state", async () => {
  const request = deferred<AssistantOnboarding>();
  vi.mocked(nyxBotApi.showSuggestions).mockReturnValue(request.promise);
  const { result, client } = setup();
  act(() => result.current.show.mutate());
  await waitFor(() => expect(client.getQueryData<AssistantOnboarding>(key)?.suggestions_hidden).toBe(false));
  expect(result.current.show.isPending).toBe(true);
  const refreshed = { ...state, suggestions_hidden: false, intro_version: 3 };
  request.resolve(refreshed);
  await waitFor(() => expect(result.current.show.isSuccess).toBe(true));
  expect(client.getQueryData(key)).toEqual(refreshed);
});

it("rolls back a failed Show, invalidates server state and exposes the error", async () => {
  const request = deferred<AssistantOnboarding>();
  vi.mocked(nyxBotApi.showSuggestions).mockReturnValue(request.promise);
  const { result, client } = setup();
  act(() => result.current.show.mutate());
  await waitFor(() => expect(client.getQueryData<AssistantOnboarding>(key)?.suggestions_hidden).toBe(false));
  const error = new Error("fixture failure");
  request.reject(error);
  await waitFor(() => expect(result.current.show.isError).toBe(true));
  expect(result.current.show.error).toBe(error);
  expect(client.getQueryData(key)).toEqual(state);
  expect(client.getQueryState(key)?.isInvalidated).toBe(true);
});

it("optimistically hides and rolls back a failed Hide", async () => {
  const request = deferred<AssistantOnboarding>();
  vi.mocked(nyxBotApi.hideSuggestions).mockReturnValue(request.promise);
  const initial = { ...state, suggestions_hidden: false };
  const { result, client } = setup(initial);
  act(() => result.current.hide.mutate());
  await waitFor(() => expect(client.getQueryData<AssistantOnboarding>(key)?.suggestions_hidden).toBe(true));
  request.reject(new Error("fixture failure"));
  await waitFor(() => expect(result.current.hide.isError).toBe(true));
  expect(client.getQueryData(key)).toEqual(initial);
});

it("serializes Hide then Undo through the same mutation scope", async () => {
  const hide = deferred<AssistantOnboarding>();
  vi.mocked(nyxBotApi.hideSuggestions).mockReturnValue(hide.promise);
  vi.mocked(nyxBotApi.showSuggestions).mockResolvedValue({ ...state, suggestions_hidden: false });
  const { result, client } = setup({ ...state, suggestions_hidden: false });
  act(() => result.current.hide.mutate());
  await waitFor(() => expect(nyxBotApi.hideSuggestions).toHaveBeenCalledOnce());
  act(() => result.current.show.mutate());
  await waitFor(() => expect(result.current.show.isPaused).toBe(true));
  expect(nyxBotApi.showSuggestions).not.toHaveBeenCalled();
  const mutations = client.getMutationCache().getAll();
  expect(mutations[0]?.options.scope).toEqual({ id: "assistant-onboarding-suggestions:owner" });
  expect(mutations[1]?.options.scope).toEqual(mutations[0]?.options.scope);
  hide.resolve(state);
  await waitFor(() => expect(result.current.show.isSuccess).toBe(true));
  expect(nyxBotApi.showSuggestions).toHaveBeenCalledOnce();
  expect(client.getQueryData<AssistantOnboarding>(key)?.suggestions_hidden).toBe(false);
});
