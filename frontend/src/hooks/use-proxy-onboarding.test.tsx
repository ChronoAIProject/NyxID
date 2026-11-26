import { act, renderHook } from "@testing-library/react";
import { beforeEach, expect, it, vi } from "vitest";
import { useAuthStore } from "@/stores/auth-store";
import type { User } from "@/types/api";
import {
  beginProxyVerification,
  FIRST_PROXY_CALL_SUCCEEDED_EVENT,
  FIRST_PROXY_CALL_SUCCEEDED_KEY,
  recordFirstProxyCallSucceeded,
  useProxyOnboarding,
} from "./use-proxy-onboarding";

function signIn(id: string | null) {
  useAuthStore.setState({
    user: id ? ({ id } as User) : null,
    isAuthenticated: id !== null,
  });
}

beforeEach(() => {
  localStorage.clear();
  signIn("user-a");
});

it("records with Dashboard unmounted and reads the result on mount and remount", () => {
  recordFirstProxyCallSucceeded("user-a");
  expect(localStorage.getItem(`${FIRST_PROXY_CALL_SUCCEEDED_KEY}:user-a`)).toBe(
    "1",
  );
  const first = renderHook(useProxyOnboarding);
  expect(first.result.current.firstProxyCallSucceeded).toBe(true);
  first.unmount();
  expect(
    renderHook(useProxyOnboarding).result.current.firstProxyCallSucceeded,
  ).toBe(true);
});

it("reads persisted state from a prior page load without needing an event", () => {
  localStorage.setItem(`${FIRST_PROXY_CALL_SUCCEEDED_KEY}:user-a`, "1");
  expect(
    renderHook(useProxyOnboarding).result.current.firstProxyCallSucceeded,
  ).toBe(true);
});

it("never adopts legacy unowned state or a bare success event", () => {
  localStorage.setItem(FIRST_PROXY_CALL_SUCCEEDED_KEY, "1");
  const { result } = renderHook(useProxyOnboarding);
  act(() =>
    window.dispatchEvent(new CustomEvent(FIRST_PROXY_CALL_SUCCEEDED_EVENT)),
  );
  expect(result.current.firstProxyCallSucceeded).toBe(false);
  expect(
    localStorage.getItem(`${FIRST_PROXY_CALL_SUCCEEDED_KEY}:user-a`),
  ).toBeNull();
});

it("isolates switches, logout and late completion to the initiating user", () => {
  const { result } = renderHook(useProxyOnboarding);
  let finish = () => {};
  act(() => {
    finish = beginProxyVerification("user-a");
  });
  expect(result.current.verifyKeyLoading).toBe(true);
  act(() => signIn("user-b"));
  expect(result.current).toEqual({
    firstProxyCallSucceeded: false,
    verifyKeyLoading: false,
  });
  act(() => {
    recordFirstProxyCallSucceeded("user-a");
    finish();
  });
  expect(result.current.firstProxyCallSucceeded).toBe(false);
  expect(
    localStorage.getItem(`${FIRST_PROXY_CALL_SUCCEEDED_KEY}:user-b`),
  ).toBeNull();
  act(() => signIn(null));
  expect(result.current.firstProxyCallSucceeded).toBe(false);
  act(() => signIn("user-a"));
  expect(result.current).toEqual({
    firstProxyCallSucceeded: true,
    verifyKeyLoading: false,
  });
});

it("counts concurrent probes, including when no hook is mounted at start", () => {
  const finishOne = beginProxyVerification("user-a");
  const finishTwo = beginProxyVerification("user-a");
  const { result } = renderHook(useProxyOnboarding);
  expect(result.current.verifyKeyLoading).toBe(true);
  act(() => {
    finishOne();
    finishOne();
  });
  expect(result.current.verifyKeyLoading).toBe(true);
  act(finishTwo);
  expect(result.current.verifyKeyLoading).toBe(false);
});

it("consumes a storage update only for the current user", () => {
  const { result } = renderHook(useProxyOnboarding);
  act(() => {
    localStorage.setItem(`${FIRST_PROXY_CALL_SUCCEEDED_KEY}:user-b`, "1");
    window.dispatchEvent(new Event("storage"));
  });
  expect(result.current.firstProxyCallSucceeded).toBe(false);
  act(() => {
    localStorage.setItem(`${FIRST_PROXY_CALL_SUCCEEDED_KEY}:user-a`, "1");
    window.dispatchEvent(new Event("storage"));
  });
  expect(result.current.firstProxyCallSucceeded).toBe(true);
});

it("retains owner-scoped session state when storage is blocked", () => {
  signIn("storage-blocked-user");
  const write = vi.spyOn(localStorage, "setItem").mockImplementation(() => {
    throw new Error("blocked");
  });
  recordFirstProxyCallSucceeded("storage-blocked-user");
  const { result } = renderHook(useProxyOnboarding);
  expect(result.current.firstProxyCallSucceeded).toBe(true);
  act(() => signIn("user-b"));
  expect(result.current.firstProxyCallSucceeded).toBe(false);
  write.mockRestore();
});
