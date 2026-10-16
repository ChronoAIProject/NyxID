import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, renderHook } from "@testing-library/react";
import type { PropsWithChildren } from "react";
import { afterEach, expect, it, vi } from "vitest";
import { FEATURE_FLAG } from "@/lib/feature-flags";
import { useAuthStore } from "@/stores/auth-store";
import type { User } from "@/types/api";
import { useRemoteMachineAdmission } from "./use-remote-machine-admission";

const originalAuth = useAuthStore.getState();
const owner = (id: string, enabled = false) =>
  ({
    id,
    capabilities: {
      enabled_features: enabled ? [FEATURE_FLAG.REMOTE_MACHINES] : [],
    },
  }) as unknown as User;
afterEach(() => {
  useAuthStore.setState(originalAuth);
  vi.unstubAllGlobals();
});
function mount() {
  const transport = vi.fn();
  vi.stubGlobal("fetch", transport);
  const client = new QueryClient();
  const wrapper = ({ children }: PropsWithChildren) => (
    <QueryClientProvider client={client}>{children}</QueryClientProvider>
  );
  return { ...renderHook(useRemoteMachineAdmission, { wrapper }), transport };
}
it.each([null, { id: "person" } as unknown as User, owner("person")])(
  "defaults to disabled for absent personal resolution %#",
  (user) => {
    useAuthStore.setState({ user, isAuthenticated: true, isLoading: false });
    const { result, transport } = mount();
    expect(result.current).toBe(false);
    expect(transport).not.toHaveBeenCalled();
  },
);
it("uses the real personal feature hook without fetching an organization", () => {
  useAuthStore.setState({
    user: owner("one", true),
    isAuthenticated: true,
    isLoading: false,
  });
  const { result, transport } = mount();
  expect(result.current).toBe(true);
  expect(transport).not.toHaveBeenCalled();
});
it("fails closed while a retained enabled user is loading or signed out", () => {
  useAuthStore.setState({
    user: owner("one", true),
    isAuthenticated: true,
    isLoading: true,
  });
  const { result } = mount();
  expect(result.current).toBe(false);
  act(() => useAuthStore.setState({ isLoading: false }));
  expect(result.current).toBe(true);
  act(() => useAuthStore.setState({ isAuthenticated: false }));
  expect(result.current).toBe(false);
});
it("does not borrow another account's flag and observes live OFF changes", () => {
  useAuthStore.setState({
    user: owner("one", true),
    isAuthenticated: true,
    isLoading: false,
  });
  const { result } = mount();
  expect(result.current).toBe(true);
  act(() => useAuthStore.setState({ user: owner("two") }));
  expect(result.current).toBe(false);
  act(() => useAuthStore.setState({ user: owner("two", true) }));
  expect(result.current).toBe(true);
  act(() => useAuthStore.setState({ user: owner("two") }));
  expect(result.current).toBe(false);
});
