import { MutationCache, QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, renderHook, waitFor } from "@testing-library/react";
import type { PropsWithChildren } from "react";
import { afterEach, beforeEach, describe, expect, it, vi } from "vitest";
import { useAuthStore } from "@/stores/auth-store";
import type { User } from "@/types/api";
import { useSetMachineAccess } from "./use-machine-access";
import { useSetNyxBotAgentGrants } from "./use-nyxbot-agents";

const transport = vi.fn<typeof fetch>();
function wrapper(client: QueryClient) {
  return ({ children }: PropsWithChildren) => <QueryClientProvider client={client}>{children}</QueryClientProvider>;
}
function deferred<T>() {
  let resolve!: (value: T) => void;
  const promise = new Promise<T>((done) => { resolve = done; });
  return { promise, resolve };
}
function useGrantWrite() {
  const mutation = useSetNyxBotAgentGrants();
  const body = { id: "agent-one", services: [], account_read: false, machines: ["machine-one"] };
  return { run: () => mutation.mutateAsync(body), send: (onSuccess: () => void) => mutation.mutate(body, { onSuccess }), body, mutation };
}
function useCapabilityWrite() {
  const mutation = useSetMachineAccess("agent-one");
  const body = { node: "machine-one", selection: { expected_revision: 7, capabilities: { shell: true, files: true, browser: false, computer: false, developer_browser: false }, saved_login_ids: [] } };
  return { run: () => mutation.mutateAsync(body), send: (onSuccess: () => void) => mutation.mutate(body, { onSuccess }), body, mutation };
}
beforeEach(() => {
  transport.mockReset();
  vi.stubGlobal("fetch", transport);
  useAuthStore.setState({ user: { id: "owner-one" } as User });
});
afterEach(() => { vi.unstubAllGlobals(); });

for (const scenario of [
  { name: "specialist membership", useWrite: useGrantWrite, endpoint: "/api/v1/assistant/nyxagent/agents/agent-one/grants" },
  { name: "machine capabilities", useWrite: useCapabilityWrite, endpoint: "/api/v1/assistant/nyxagent/agents/agent-one/machines/machine-one" },
]) {
  describe(scenario.name, () => {
    it("binds the owner before deferred mutation middleware", async () => {
      const gate = deferred<void>();
      const client = new QueryClient({ mutationCache: new MutationCache({ onMutate: () => gate.promise }) });
      const { result } = renderHook(() => scenario.useWrite(), { wrapper: wrapper(client) });
      let pending!: Promise<unknown>;
      act(() => { pending = result.current.run(); });
      const rejected = expect(pending).rejects.toThrow("account changed");
      act(() => useAuthStore.setState({ user: { id: "owner-two" } as User }));
      await act(async () => { gate.resolve(); await rejected; });
      expect(transport).not.toHaveBeenCalled();
      client.clear();
    });

    it("rejects a response arriving after the owner switches", async () => {
      const response = deferred<Response>();
      transport.mockReturnValueOnce(response.promise);
      const client = new QueryClient();
      const { result } = renderHook(() => scenario.useWrite(), { wrapper: wrapper(client) });
      let pending!: Promise<unknown>;
      act(() => { pending = result.current.run(); });
      const rejected = expect(pending).rejects.toThrow("account changed");
      await waitFor(() => expect(transport).toHaveBeenCalledOnce());
      act(() => useAuthStore.setState({ user: { id: "owner-two" } as User }));
      await act(async () => { response.resolve(new Response(null, { status: 204 })); await rejected; });
      expect(result.current.mutation.data).toBeUndefined();
      client.clear();
    });

    it("preserves the original endpoint and mutation caller contract", async () => {
      transport.mockResolvedValueOnce(new Response(null, { status: 204 }));
      const client = new QueryClient();
      const { result } = renderHook(() => scenario.useWrite(), { wrapper: wrapper(client) });
      const success = vi.fn();
      act(() => result.current.send(success));
      await waitFor(() => expect(success).toHaveBeenCalledOnce());
      expect(transport.mock.calls[0]?.[0]).toBe(scenario.endpoint);
      expect(transport.mock.calls[0]?.[1]?.method).toBe("PUT");
      expect(result.current.mutation.variables).toEqual(result.current.body);
      client.clear();
    });
  });
}
