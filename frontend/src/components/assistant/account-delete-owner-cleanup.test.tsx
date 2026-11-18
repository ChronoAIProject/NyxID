import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, it, vi } from "vitest";
import { useAuthStore } from "@/stores/auth-store";
import { installAccountSessionSync } from "@/lib/account-session-sync";
import { AssistantAccountDeleteDialog } from "./assistant-account-delete-dialog";
import type { User } from "@/types/api";
const A = "00000000-0000-4000-8000-000000000001";
const B = "00000000-0000-4000-8000-000000000002";
afterEach(() => vi.unstubAllGlobals());
it.each(["B", "401", "none"])("keeps accepted Assistant deletion cleanup scoped to A with %s evidence", async (outcome) => {
  useAuthStore.getState().setUser({ id: A, email: "a@example.test" } as User);
  const hints: Array<string | null> = [];
  let hint!: () => void;
  vi.stubGlobal("BroadcastChannel", class {
    set onmessage(fn: (event: { data: { userId: string; nonce: string } }) => void) {
      hint = () => fn({ data: { userId: B, nonce: crypto.randomUUID() } });
    }
    postMessage(value: { userId: string | null }) { hints.push(value.userId); }
    close() {}
  });
  let verifying = false;
  let release!: () => void;
  const network = vi.fn(async (url: string) => {
    if (url === "/api/v1/users/me") {
      if (!verifying) return Response.json({ id: A, email: "a@example.test" });
      return new Promise<Response>((resolve) => { release = () => resolve(outcome === "401" ? Response.json({}, { status: 401 }) : Response.json({ id: B, email: "b@example.test" })); });
    }
    if (url === "/api/v1/users/me/authorization") return Response.json({ id: A, is_active: true, mfa_enabled: false, display_name_length: null, avatar_url_length: null, updated_at: "2026-01-01T00:00:00.000Z" });
    if (url === "/api/v1/assistant/actions/org/account/delete") {
      if (outcome !== "none") { verifying = true; hint(); }
      return Response.json({ resource: { userId: A }, replayed: false, erasure: { status: "accepted", accepted_at: "2026-10-19T00:00:00Z", completed_at: null, domains: [] } });
    }
    throw Error(`Unexpected synthetic request ${url}`);
  });
  vi.stubGlobal("fetch", network);
  const stop = installAccountSessionSync();
  const onComplete = vi.fn();
  try {
    render(<AssistantAccountDeleteDialog open onOpenChange={vi.fn()} actionRequestId="fixture" onComplete={onComplete} />);
    await userEvent.type(screen.getByLabelText("Account email"), "a@example.test");
    await userEvent.click(screen.getByRole("button", { name: "Delete account" }));
    const button = await screen.findByRole("button", { name: "Continue" });
    expect(screen.getByRole("dialog")).toHaveAccessibleName("Account deletion accepted");
    await userEvent.click(button);
    expect(onComplete).toHaveBeenCalledExactlyOnceWith(A);
    if (outcome === "none") {
      expect(useAuthStore.getState().user).toBeNull();
      expect(hints).toEqual([null]);
    } else {
      await waitFor(() => expect(release).toBeTypeOf("function"));
      expect(useAuthStore.getState().user?.id).toBe(A);
      expect(hints).toEqual([]);
      await act(async () => release());
      await waitFor(() => expect(useAuthStore.getState().isIdentityVerifying).toBe(false));
      expect(useAuthStore.getState().user?.id).toBe(outcome === "B" ? B : undefined);
      expect(hints).toEqual([outcome === "B" ? B : null]);
    }
    expect(network.mock.calls.filter(([url]) => url === "/api/v1/auth/logout")).toHaveLength(0);
  } finally { stop(); }
});
