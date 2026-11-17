import { afterEach, expect, it, vi } from "vitest";
import { createStore } from "zustand/vanilla";

const stops: (() => void)[] = [];
afterEach(() => {
  for (const stop of stops.splice(0)) stop();
  vi.doUnmock("@/stores/auth-store");
  vi.resetModules();
  vi.unstubAllGlobals();
});

it.each(["B", null])("converges two tabs on %s without re-verifying the originating tab", async (target) => {
  const channels: TestChannel[] = [];
  class TestChannel {
    onmessage?: (event: { data: unknown }) => void;
    postMessage = vi.fn((data: unknown) => {
      for (const channel of channels) {
        if (channel !== this) channel.onmessage?.({ data });
      }
    });
    constructor() { channels.push(this); }
    close() {}
  }
  vi.stubGlobal("BroadcastChannel", TestChannel);
  const user = (id: string | null) => id === null ? null : { id };
  const makeTab = () => {
    const verifySession = vi.fn(async () => {
      await Promise.resolve();
      store.setState({ user: user(target) });
      return true;
    });
    const store = createStore(() => ({
      user: user("A"),
      isIdentityVerifying: false,
      verifySession,
    }));
    return store;
  };
  const origin = makeTab();
  const receiver = makeTab();

  // Separate module instances retain separate stores, as browser tabs do.
  vi.resetModules();
  vi.doMock("@/stores/auth-store", () => ({ useAuthStore: origin }));
  stops.push((await import("./account-session-sync")).installAccountSessionSync());
  vi.resetModules();
  vi.doMock("@/stores/auth-store", () => ({ useAuthStore: receiver }));
  stops.push((await import("./account-session-sync")).installAccountSessionSync());

  origin.setState({ user: user(target) });
  await vi.waitFor(() => expect(receiver.getState().user).toEqual(user(target)));
  expect(channels[0]!.postMessage).toHaveBeenCalledExactlyOnceWith(
    expect.objectContaining({ userId: target }),
  );
  expect(channels[1]!.postMessage).toHaveBeenCalledExactlyOnceWith(
    expect.objectContaining({ userId: target }),
  );
  expect(receiver.getState().verifySession).toHaveBeenCalledOnce();
  expect(origin.getState().verifySession).not.toHaveBeenCalled();
  expect(origin.getState().user).toEqual(user(target));
});
