import { act, renderHook, waitFor } from "@testing-library/react";
import { expect, it, vi } from "vitest";
import { useWebAuthDeviceLogin } from "@/hooks/use-auth-device";
import { useAuthStore } from "@/stores/auth-store";
import { installAccountSessionSync } from "./account-session-sync";

it.each(["ordinary", "newer signed-out verification", "cancelled"])(
  "scoped QR profile respects %s",
  async (schedule) => {
    useAuthStore.getState().setUser(null);
    const posts: Array<string | null> = [];
    let hint!: (id: string | null) => void;
    vi.stubGlobal(
      "BroadcastChannel",
      class {
        set onmessage(receive: (event: { data: unknown }) => void) {
          hint = (userId) =>
            receive({ data: { userId, nonce: crypto.randomUUID() } });
        }
        postMessage(message: { userId: string | null }) {
          posts.push(message.userId);
        }
        close() {}
      },
    );
    const profiles: Array<(response: Response) => void> = [];
    const network = vi.fn(async (url: string) => {
      if (url === "/api/v1/auth/device/request")
        return Response.json({
          device_code: "fixture",
          user_code: "TEST-CODE",
          verification_uri: "https://example.test/",
          verification_uri_complete: "https://example.test/",
          expires_in: 60,
          interval: 1,
        });

      if (url === "/api/v1/auth/device/poll-web")
        return Response.json({ ok: true });
      if (url === "/api/v1/users/me")
        return new Promise<Response>((resolve) => profiles.push(resolve));
      throw Error("Unexpected recording-transport call");
    });
    vi.stubGlobal("fetch", network);
    const stop = installAccountSessionSync();
    const qr = renderHook(useWebAuthDeviceLogin);
    try {
      await act(async () => qr.result.current.start());
      await waitFor(() => expect(profiles).toHaveLength(1), { timeout: 3000 });
      expect(qr.result.current.phase).toBe("restoring");
      if (schedule === "newer signed-out verification") {
        act(() => hint("B"));
        await waitFor(() => expect(profiles).toHaveLength(2));
        act(() => hint(null));
        await waitFor(() => expect(profiles).toHaveLength(3));
        await act(async () => {
          profiles[1]!(Response.json({ id: "B", email: "old@example.test" }));
          profiles[2]!(Response.json({}, { status: 401 }));
        });
        await waitFor(() =>
          expect(useAuthStore.getState().isIdentityVerifying).toBe(false),
        );
        expect(useAuthStore.getState().user).toBeNull();
        expect(posts).toEqual([]);
      } else if (schedule === "cancelled") {
        act(() => qr.result.current.close());
      }
      await act(async () =>
        profiles[0]!(Response.json({ id: "B", email: "old@example.test" })),
      );
      expect(
        network.mock.calls.filter(([url]) => url.endsWith("/poll-web")),
      ).toHaveLength(1);
      expect({
        phase: qr.result.current.phase,
        user: useAuthStore.getState().user?.id ?? null,
        posts,
      }).toEqual(
        schedule === "ordinary"
          ? { phase: "success", user: "B", posts: ["B"] }
          : schedule === "cancelled"
            ? { phase: "idle", user: null, posts: [] }
            : { phase: "restore_error", user: null, posts: [] },
      );
      if (schedule === "newer signed-out verification") {
        act(() => qr.result.current.retryProfile());
        await waitFor(() => expect(profiles).toHaveLength(4));
        await act(async () =>
          profiles[3]!(Response.json({ id: "B", email: "fresh@example.test" })),
        );
        expect({
          phase: qr.result.current.phase,
          user: useAuthStore.getState().user?.id,
          posts,
        }).toEqual({ phase: "success", user: "B", posts: ["B"] });
        expect(
          network.mock.calls.filter(([url]) => url.endsWith("/poll-web")),
        ).toHaveLength(1);
      }
    } finally {
      qr.unmount();
      stop();
      vi.unstubAllGlobals();
    }
  },
);

it.each(["ordinary", "newer signed-out verification", "cancelled"])(
  "ephemeral app-approval profile respects %s",
  async (schedule) => {
    useAuthStore.getState().setUser(null);
    const posts: Array<string | null> = [];
    let hint!: (id: string | null) => void;
    vi.stubGlobal(
      "BroadcastChannel",
      class {
        set onmessage(receive: (event: { data: unknown }) => void) {
          hint = (userId) =>
            receive({ data: { userId, nonce: crypto.randomUUID() } });
        }
        postMessage(message: { userId: string | null }) {
          posts.push(message.userId);
        }
        close() {}
      },
    );
    const profiles: Array<(response: Response) => void> = [];
    vi.stubGlobal(
      "fetch",
      vi.fn((url: string) => {
        if (url !== "/api/v1/users/me")
          throw Error("Unexpected recording-transport call");
        return new Promise<Response>((resolve) => profiles.push(resolve));
      }),
    );
    const stop = installAccountSessionSync();
    let current = true;
    try {
      // The keep_signed_in app approval uses these exact scoped arguments.
      const restored = useAuthStore
        .getState()
        .checkAuth({ ephemeral: true, isCurrent: () => current });
      await waitFor(() => expect(profiles).toHaveLength(1));
      if (schedule === "newer signed-out verification") {
        hint("B");
        await waitFor(() => expect(profiles).toHaveLength(2));
        hint(null);
        await waitFor(() => expect(profiles).toHaveLength(3));
        profiles[1]!(Response.json({ id: "B", email: "old@example.test" }));
        profiles[2]!(Response.json({}, { status: 401 }));
        await waitFor(() =>
          expect(useAuthStore.getState().isIdentityVerifying).toBe(false),
        );
      } else if (schedule === "cancelled") current = false;
      profiles[0]!(Response.json({ id: "B", email: "old@example.test" }));
      // Ignoring this boolean in the approval flow must still be safe: no
      // rejection and no retained browser sign-in from an obsolete profile.
      await expect(restored).resolves.toBe(schedule === "ordinary");
      expect({
        user: useAuthStore.getState().user?.id ?? null,
        signedIn: useAuthStore.getState().isAuthenticated,
        posts,
      }).toEqual(
        schedule === "ordinary"
          ? { user: "B", signedIn: true, posts: ["B"] }
          : { user: null, signedIn: false, posts: [] },
      );
    } finally {
      stop();
      vi.unstubAllGlobals();
    }
  },
);
