import { act, render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { useAuthStore } from "@/stores/auth-store";
import type { User } from "@/types/api";
import { CliPairPage } from "@/pages/cli-pair";
import { ApiKeyRotateConfirm } from "./confirm-panels";
import { invalidateAccountEvidence, settleAccountEvidence, transitionAssistantIdentity } from "@/lib/assistant/identity";
const { record } = vi.hoisted(() => ({ record: vi.fn() }));
vi.mock("@/lib/api-client", async (importOriginal) => ({
  ...(await importOriginal<typeof import("@/lib/api-client")>()),
  api: { post: record, get: vi.fn().mockResolvedValue({ status: "claimed" }) },
}));
beforeEach(() => { record.mockReset(); settleAccountEvidence(); transitionAssistantIdentity("A"); });
afterEach(() => settleAccountEvidence());
it.each(["same", "different", "none"])("retains an accepted rotation for its initiating owner: %s evidence", async (evidence) => {
  const delivered = vi.fn();
  let accept!: () => void;
  record.mockImplementation(async (url: string) => {
    if (url.endsWith("/reserve-action")) return {};
    if (url.endsWith("/rotate")) return new Promise((resolve) => {
      accept = () => resolve({ id: "successor-fixture", full_key: "synthetic-once" });
    });
    throw Error("unexpected local call");
  });
  render(<ApiKeyRotateConfirm pairingId="fixture" onSuccess={delivered} prefill={{ resource_id: "fixture", display_name: "Fixture" }} />);
  await userEvent.click(screen.getByRole("button", { name: "Rotate key" }));
  await waitFor(() => expect(accept).toBeTypeOf("function"));
  if (evidence !== "none") act(() => invalidateAccountEvidence());
  await act(async () => { accept(); await Promise.resolve(); });
  if (evidence !== "none") {
    expect(delivered).not.toHaveBeenCalled();
    expect(screen.queryByText("synthetic-once")).not.toBeInTheDocument();
    // An accepted result must never be generated again while waiting.
    expect(screen.getByRole("button", { name: "Rotate key" })).toBeDisabled();
    act(() => { if (evidence === "different") transitionAssistantIdentity("B"); settleAccountEvidence(); });
  }
  if (evidence === "different") expect(delivered).not.toHaveBeenCalled();
  else await waitFor(() => expect(delivered).toHaveBeenCalledExactlyOnceWith({ kind: "api-key-rotate", resource_id: "successor-fixture", full_key: "synthetic-once", platform: undefined }));
  act(() => { invalidateAccountEvidence(); settleAccountEvidence(); });
  expect(delivered).toHaveBeenCalledTimes(evidence === "different" ? 0 : 1);
  expect(record.mock.calls.filter(([url]) => url.endsWith("/rotate"))).toHaveLength(1);
});

it.each(["same", "different", "none"])("real rotation handoff and CLI ack preserve the accepted result with %s evidence", async (evidence) => {
  useAuthStore.getState().setUser({ id: "A", email: "a@example.test" } as User);
  useAuthStore.setState({ isLoading: false });
  let accept!: () => void;
  record.mockImplementation(async (url: string) => {
    if (url === "/cli-pairings/claim") return { id: "fixture", kind: "api-key-rotate", prefill: { resource_id: "old", display_name: "Fixture" }, resumed: false, action_started: false };
    if (url.endsWith("/reserve-action")) return {};
    if (url.endsWith("/rotate")) return new Promise(resolve => { accept = () => resolve({ id: "successor", full_key: "synthetic-once" }); });
    if (url.endsWith("/complete")) return {};
    throw Error(`Unexpected fixture request ${url}`);
  });
  render(<CliPairPage />);
  await userEvent.type(screen.getByLabelText(/Pairing code/i), "abcd-1234");
  await userEvent.click(screen.getByRole("button", { name: "Continue" }));
  await userEvent.click(await screen.findByRole("button", { name: "Rotate key" }));
  await waitFor(() => expect(accept).toBeTypeOf("function"));
  if (evidence !== "none") act(() => invalidateAccountEvidence());
  await act(async () => { accept(); await Promise.resolve(); });
  if (evidence !== "none") {
    expect(record.mock.calls.filter(([url]) => url.endsWith("/complete"))).toHaveLength(0);
    act(() => { if (evidence === "different") useAuthStore.getState().setUser({ id: "B", email: "b@example.test" } as User); settleAccountEvidence(); });
  }
  if (evidence !== "different") {
    await screen.findByRole("heading", { name: "API key rotated" });
    expect(record.mock.calls.filter(([url]) => url.endsWith("/complete"))).toEqual([["/cli-pairings/fixture/complete", { ack: { acknowledged: true, resource_id: "successor" } }]]);
    act(() => { invalidateAccountEvidence(); settleAccountEvidence(); });
    expect(record.mock.calls.filter(([url]) => url.endsWith("/complete"))).toHaveLength(1);
  } else {
    expect(record.mock.calls.filter(([url]) => url.endsWith("/complete"))).toHaveLength(0);
    expect(screen.queryByRole("heading", { name: "API key rotated" })).not.toBeInTheDocument();
    expect(document.body.innerHTML).not.toContain("synthetic-once");
  }
  expect(record.mock.calls.filter(([url]) => url.endsWith("/rotate"))).toHaveLength(1);
});
