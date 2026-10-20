import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { expect, it, vi } from "vitest";
import { api } from "@/lib/api-client";
import { AccountErasureProgress } from "./account-erasure-progress";

vi.mock("@/lib/api-client", () => ({
  api: { get: vi.fn(), post: vi.fn() },
  ApiError: class ApiError extends Error {},
}));

it("starts both pagination streams at the first page after an account route switch", async () => {
  const user = userEvent.setup();
  const requested: string[] = [];
  vi.mocked(api.get).mockImplementation(async (url) => {
    requested.push(url);
    return {
      erasure: { status: "pending", accepted_at: "2026-10-19T00:00:00Z", completed_at: null, domains: [] },
      effects: [], node_drains: [],
      next_after: "effect-page-two", next_node_after: "node-page-two",
    } as never;
  });
  const client = new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: 0 } } });
  const view = render(<QueryClientProvider client={client}><AccountErasureProgress userId="first-owner" /></QueryClientProvider>);
  await user.click(await screen.findByRole("button", { name: "Next effects page" }));
  await waitFor(() => expect(requested.some((url) => url.includes("after=effect-page-two"))).toBe(true));
  await user.click(screen.getByRole("button", { name: "Next nodes page" }));
  await waitFor(() => expect(requested.some((url) => url.includes("nodes_after=node-page-two") && url.includes("after=effect-page-two"))).toBe(true));
  view.rerender(<QueryClientProvider client={client}><AccountErasureProgress userId="second-owner" /></QueryClientProvider>);
  await waitFor(() => expect(requested).toContain("/admin/users/second-owner/erasure?"));
  expect(requested.filter((url) => url.includes("second-owner"))).toEqual(["/admin/users/second-owner/erasure?"]);
  expect(screen.queryByRole("button", { name: "First effects page" })).not.toBeInTheDocument();
  expect(screen.queryByRole("button", { name: "First nodes page" })).not.toBeInTheDocument();
  view.unmount();
  client.clear();
});


it("binds an operator retry to the displayed job and domain round", async () => {
  const user = userEvent.setup();
  const jobId = "b966a558-bbf1-4baa-a091-0370b6e3d666";
  vi.mocked(api.get).mockResolvedValue({
    erasure: { job_id: jobId, status: "operator_action", accepted_at: "2026-10-19T00:00:00Z", completed_at: null,
      domains: [{ domain: "machine", retry_generation: 3, status: "operator_action", reason: "machine_lifecycle_reconciliation_required", completed_at: null }] },
    effects: [], node_drains: [], next_after: null, next_node_after: null,
  } as never);
  vi.mocked(api.post).mockResolvedValue({} as never);
  const client = new QueryClient({ defaultOptions: { queries: { retry: false, gcTime: 0 } } });
  const view = render(<QueryClientProvider client={client}><AccountErasureProgress userId="owner" /></QueryClientProvider>);
  await user.click(await screen.findByRole("button", { name: "Retry domain" }));
  await waitFor(() => expect(api.post).toHaveBeenCalledWith("/admin/users/owner/erasure/retry", {
    domain: "machine", job_id: jobId, expected_retry_generation: 3, idempotency_key: expect.any(String),
  }));
  const request = vi.mocked(api.post).mock.calls.at(-1)?.[1] as { idempotency_key: string };
  expect(request.idempotency_key).toMatch(/^[0-9a-f]{8}-[0-9a-f-]{27}$/);
  view.unmount();
  client.clear();
});
