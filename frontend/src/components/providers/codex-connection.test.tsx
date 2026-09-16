import type { ReactNode } from "react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, render, renderHook, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, describe, expect, it, vi } from "vitest";
import { CodexConnectionSection } from "./codex-connection";
import { useCodexConnection, useVerifyCodexConnection } from "@/hooks/use-codex-connection";
import type { CodexConnection } from "@/schemas/codex-connection";

const { get, post } = vi.hoisted(() => ({ get: vi.fn(), post: vi.fn() }));
vi.mock("@/lib/api-client", () => ({ apiClient: get, api: { post } }));
vi.mock("@tanstack/react-router", () => ({
  Link: ({ children }: { children: ReactNode }) => <a>{children}</a>,
}));

const queryKey = ["codex-connection"];
const saved: CodexConnection = {
  account_id: "11111111-1111-4111-8111-111111111111", account_email: "reviewed@example.test",
  provider_id: "22222222-2222-4222-8222-222222222222", provider_slug: "openai",
  connection: { id: "33333333-3333-4333-8333-333333333333", state_version: 1 },
  status: "usable", service_id: "44444444-4444-4444-8444-444444444444", feature: "openai_responses",
};
const newer = { ...saved, connection: { ...saved.connection!, state_version: 2 }, status: "saved" as const };

function setup() {
  const client = new QueryClient({ defaultOptions: { queries: { retry: false }, mutations: { retry: false } } });
  client.setQueryData(queryKey, saved);
  const wrapper = ({ children }: { children: ReactNode }) => <QueryClientProvider client={client}>{children}</QueryClientProvider>;
  return { client, wrapper };
}

describe("Codex connection verification", () => {
  beforeEach(() => { vi.resetAllMocks(); });

  it("submits the reviewed version and account after a background connection change", async () => {
    const { client, wrapper } = setup();
    post.mockResolvedValue({ ...saved, status: "usable" });
    render(<CodexConnectionSection />, { wrapper });
    const user = userEvent.setup();
    await user.click(screen.getByRole("button", { name: "Codex connection" }));
    await user.click(screen.getByRole("button", { name: "Verify connection" }));
    act(() => client.setQueryData(queryKey, { ...newer, account_email: "changed@example.test" }));
    expect(screen.getByRole("dialog")).toHaveTextContent(saved.account_email);
    expect(screen.getByRole("dialog")).not.toHaveTextContent("changed@example.test");
    await user.click(screen.getByRole("button", { name: /^Verify$/ }));
    await waitFor(() => expect(post).toHaveBeenCalledExactlyOnceWith("/providers/codex-connection/verify", {
      connection: saved.connection, model: "gpt-4.1-mini",
    }));
    expect(client.getQueryData<CodexConnection>(queryKey)?.connection?.state_version).toBe(2);
  });

  it("cancels a stale usable GET before verification and retains saved on failure", async () => {
    const { client, wrapper } = setup();
    let resolveGet!: (value: CodexConnection) => void;
    let rejectPost!: (error: Error) => void;
    get.mockImplementation(() => new Promise((resolve) => { resolveGet = resolve; }));
    post.mockImplementation(() => new Promise((_, reject) => { rejectPost = reject; }));
    const { result } = renderHook(() => ({ query: useCodexConnection(true), verify: useVerifyCodexConnection() }), { wrapper });
    act(() => { void result.current.query.refetch(); });
    await waitFor(() => expect(get).toHaveBeenCalledTimes(1));
    act(() => result.current.verify.mutate(saved));
    await waitFor(() => expect(post).toHaveBeenCalledTimes(1));
    expect(get.mock.calls[0]?.[1]?.signal.aborted).toBe(true);
    await act(async () => { resolveGet(saved); });
    expect(client.getQueryData<CodexConnection>(queryKey)?.status).toBe("saved");
    await act(async () => { rejectPost(new Error("fixture outage")); });
    await waitFor(() => expect(result.current.verify.isError).toBe(true));
    expect(result.current.query.data?.status).toBe("saved");
    expect(get).toHaveBeenCalledTimes(1);
  });

  it("does not overwrite a replacement connection with an older successful verification", async () => {
    const { client, wrapper } = setup();
    let resolvePost!: (value: CodexConnection) => void;
    post.mockImplementation(() => new Promise((resolve) => { resolvePost = resolve; }));
    const { result } = renderHook(() => useVerifyCodexConnection(), { wrapper });
    act(() => result.current.mutate(saved));
    await waitFor(() => expect(post).toHaveBeenCalledTimes(1));
    act(() => client.setQueryData(queryKey, newer));
    await act(async () => { resolvePost(saved); });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(client.getQueryData(queryKey)).toEqual(newer);
  });
});
