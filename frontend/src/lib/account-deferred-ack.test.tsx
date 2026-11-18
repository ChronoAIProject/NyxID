import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { it, expect, vi } from "vitest";
import { CliPairPage } from "@/pages/cli-pair";
import { useAuthStore } from "@/stores/auth-store";
import type { User } from "@/types/api";
import { installAccountSessionSync } from "./account-session-sync";
import {
  isAccountEvidencePending,
  settleAccountEvidence,
} from "./assistant/identity";

it.each([
  "confirmed A",
  "confirmed B",
  "leaving during verification",
  "leaving then confirmed A",
  "disposing during verification",
])(
  "accepted handoff waits for real profile confirmation on %s",
  async (schedule) => {
    useAuthStore
      .getState()
      .setUser({ id: "A", email: "a@example.test" } as User);
    useAuthStore.setState({ isLoading: false });
    let hint!: () => void;
    vi.stubGlobal(
      "BroadcastChannel",
      class {
        set onmessage(receive: (event: { data: unknown }) => void) {
          hint = () =>
            receive({ data: { userId: "B", nonce: crypto.randomUUID() } });
        }
        postMessage() {}
        close() {}
      },
    );
    const calls: string[] = [];
    let accept!: (r: Response) => void;
    let verified!: (r: Response) => void;
    vi.stubGlobal("fetch", async (url: string) => {
      calls.push(url);
      if (url.endsWith("/claim"))
        return Response.json({
          id: "fixture",
          kind: "api-key-rotate",
          prefill: { resource_id: "old", display_name: "Fixture" },
          resumed: false,
          action_started: false,
        });
      if (url.endsWith("/poll")) return Response.json({ status: "claimed" });
      if (url.endsWith("/complete") || url.endsWith("/reserve-action"))
        return Response.json({});
      if (url.endsWith("/rotate"))
        return new Promise<Response>((resolve) => {
          accept = resolve;
        });
      if (url.endsWith("/users/me"))
        return new Promise<Response>((resolve) => {
          verified = resolve;
        });
      throw Error("Unexpected recording-transport call");
    });
    const stop = installAccountSessionSync();
    const view = render(<CliPairPage />);
    try {
      await userEvent.type(screen.getByLabelText(/Pairing code/i), "abcd-1234");
      await userEvent.click(screen.getByRole("button", { name: "Continue" }));
      await userEvent.click(
        await screen.findByRole("button", { name: "Rotate key" }),
      );
      await waitFor(() => expect(accept).toBeTypeOf("function"));
      act(() => hint());
      await waitFor(() => expect(verified).toBeTypeOf("function"));
      await act(async () =>
        accept(
          Response.json({ id: "fixture-result", full_key: "synthetic-only" }),
        ),
      );
      expect(calls.filter((url) => url.endsWith("/complete"))).toHaveLength(0);
      expect(isAccountEvidencePending()).toBe(true);
      if (schedule.startsWith("leaving")) {
        const leaving = new Event("pagehide");
        Object.defineProperty(leaving, "persisted", { value: false });
        await act(async () => {
          window.dispatchEvent(leaving);
          await new Promise((resolve) => setTimeout(resolve, 20));
        });
        expect(isAccountEvidencePending()).toBe(true);
        expect(calls.filter((url) => url.endsWith("/complete"))).toHaveLength(
          0,
        );
      } else if (schedule === "disposing during verification") {
        await act(async () => stop());
        expect(isAccountEvidencePending()).toBe(true);
        expect(calls.filter((url) => url.endsWith("/complete"))).toHaveLength(
          0,
        );
        // Even a late same-owner response cannot authorize a disposed verifier.
        await act(async () =>
          verified(Response.json({ id: "A", email: "a@example.test" })),
        );
        expect(isAccountEvidencePending()).toBe(true);
      }
      const confirmedA =
        schedule === "confirmed A" || schedule === "leaving then confirmed A";
      if (confirmedA || schedule === "confirmed B") {
        const id = confirmedA ? "A" : "B";
        await act(async () =>
          verified(Response.json({ id, email: `${id}@example.test` })),
        );
        await waitFor(() => expect(isAccountEvidencePending()).toBe(false));
        if (confirmedA)
          await screen.findByRole("heading", { name: "API key rotated" });
        else {
          expect(
            screen.queryByRole("heading", { name: "API key rotated" }),
          ).not.toBeInTheDocument();
          expect(document.body.innerHTML).not.toContain("synthetic-only");
        }
      }
      expect(calls.filter((url) => url.endsWith("/rotate"))).toHaveLength(1);
      expect(calls.filter((url) => url.endsWith("/complete"))).toHaveLength(
        confirmedA ? 1 : 0,
      );
    } finally {
      view.unmount();
      stop();
      vi.unstubAllGlobals();
      // Reset the test module after consumers unmount; teardown itself cannot confirm.
      settleAccountEvidence();
    }
  },
);
