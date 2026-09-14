import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { act, fireEvent, render, screen, waitFor } from "@testing-library/react";
import { afterEach, beforeEach, expect, it, vi } from "vitest";
import { ConnectionValidationCard } from "./connection-validation-card";

const { post } = vi.hoisted(() => ({ post: vi.fn() }));
vi.mock("@/lib/api-client", () => ({ api: { post } }));

function mount(active = true) {
  const client = new QueryClient({ defaultOptions: { mutations: { retry: false } } });
  return render(
    <QueryClientProvider client={client}>
      <ConnectionValidationCard serviceId="service-id" active={active} />
    </QueryClientProvider>,
  );
}

const response = () => ({
  user_service_id: "service-id",
  validator_id: "github_user_v1",
  validator_version: 1,
  outcome: "authenticated",
  claim: "GitHub accepted this credential for the authenticated user endpoint.",
  checked_at: new Date().toISOString(),
  valid_until: new Date(Date.now() + 300_000).toISOString(),
  reason_code: "authenticated",
});

beforeEach(() => vi.clearAllMocks());
afterEach(() => vi.useRealTimers());

it("only probes on an explicit click and renders the bounded claim", async () => {
  post.mockResolvedValue(response());
  mount();
  expect(post).not.toHaveBeenCalled();
  fireEvent.click(screen.getByRole("button", { name: "Check connection" }));
  expect(await screen.findByText("Connection checked")).toBeInTheDocument();
  expect(post).toHaveBeenCalledExactlyOnceWith("/keys/service-id/validate", { force: false });
  expect(screen.getByText(response().claim)).toBeInTheDocument();
  fireEvent.click(screen.getByRole("button", { name: "Check connection" }));
  await waitFor(() => expect(post).toHaveBeenLastCalledWith("/keys/service-id/validate", { force: true }));
});

it("disables validation for a disabled service", () => {
  mount(false);
  const button = screen.getByRole("button", { name: "Check connection" });
  expect(button).toBeDisabled();
  fireEvent.click(button);
  expect(post).not.toHaveBeenCalled();
});

it("shows upgrade-required outcomes and does not retry failed requests", async () => {
  post.mockResolvedValueOnce({ ...response(), outcome: "unsupported", reason_code: "node_agent_upgrade_required" });
  mount();
  fireEvent.click(screen.getByRole("button", { name: "Check connection" }));
  expect(await screen.findByText("Upgrade the connected node agent to check this connection.")).toBeInTheDocument();
  post.mockRejectedValueOnce(new Error("You do not have proxy access to this service"));
  fireEvent.click(screen.getByRole("button", { name: "Check connection" }));
  expect(await screen.findByRole("alert")).toHaveTextContent("You do not have proxy access");
  expect(post).toHaveBeenCalledTimes(2);
});

it("expires display evidence without sending another probe", async () => {
  vi.useFakeTimers();
  post.mockResolvedValue(response());
  mount();
  fireEvent.click(screen.getByRole("button", { name: "Check connection" }));
  await act(async () => { await vi.advanceTimersByTimeAsync(10); });
  expect(screen.getByText("Connection checked")).toBeInTheDocument();
  await act(async () => { await vi.advanceTimersByTimeAsync(300_000); });
  expect(screen.getByText("Check expired")).toBeInTheDocument();
  expect(post).toHaveBeenCalledTimes(1);
});


it.each([
  ["credential_unavailable", "The stored credential could not be used. Reconnect this service."],
  ["attempt_superseded", "The connection changed while it was being checked. Check again."],
])("shows actionable guidance for %s without retrying automatically", async (reason, message) => {
  const checked = new Date().toISOString();
  post.mockResolvedValue({
    ...response(), outcome: "transport_unknown", reason_code: reason,
    checked_at: checked, valid_until: checked,
  });
  mount();
  fireEvent.click(screen.getByRole("button", { name: "Check connection" }));
  expect(await screen.findByText(message)).toBeInTheDocument();
  expect(post).toHaveBeenCalledTimes(1);
});
