import { act, renderHook, waitFor } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import type { PropsWithChildren } from "react";
import { beforeEach, describe, expect, it, vi } from "vitest";
import {
  useSandboxBillingSettings,
  useUpdateSandboxBillingSettings,
} from "./use-sandbox-billing-settings";
const mock = vi.hoisted(() => ({ get: vi.fn(), put: vi.fn() }));
vi.mock("@/lib/api-client", () => ({ api: mock }));
const id = "11111111-1111-4111-8111-111111111111";
const producer = "22222222-2222-4222-8222-222222222222";
function setup() {
  const client = new QueryClient({
    defaultOptions: { queries: { retry: false }, mutations: { retry: false } },
  });
  return {
    client,
    wrapper: ({ children }: PropsWithChildren) => (
      <QueryClientProvider client={client}>{children}</QueryClientProvider>
    ),
  };
}
describe("service-owned Sandbox settings API", () => {
  beforeEach(() => {
    mock.get.mockReset();
    mock.put.mockReset();
  });
  it("reads settings and preserves an unset producer", async () => {
    const settings = {
      service_id: id,
      authorized_producer_service_account_id: null,
      revision: 0,
    };
    mock.get.mockResolvedValue(settings);
    const { wrapper } = setup();
    const { result } = renderHook(() => useSandboxBillingSettings(id), {
      wrapper,
    });
    await waitFor(() => expect(result.current.isSuccess).toBe(true));
    expect(result.current.data).toEqual(settings);
    expect(mock.get).toHaveBeenCalledWith(
      `/admin/billing/services/${id}/sandbox`,
    );
  });
  it("writes the selected producer with the revision and updates readback", async () => {
    const settings = {
      service_id: id,
      authorized_producer_service_account_id: producer,
      revision: 3,
    };
    mock.put.mockResolvedValue(settings);
    const { client, wrapper } = setup();
    const { result } = renderHook(() => useUpdateSandboxBillingSettings(id), {
      wrapper,
    });
    const input = {
      authorized_producer_service_account_id: producer,
      expected_revision: 2,
    };
    await act(async () => {
      await result.current.mutateAsync(input);
    });
    expect(mock.put).toHaveBeenCalledWith(
      `/admin/billing/services/${id}/sandbox`,
      input,
    );
    expect(client.getQueryData(["admin", "billing", "sandbox", id])).toEqual(
      settings,
    );
  });
});
