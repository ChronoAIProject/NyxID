import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { expect, it, vi } from "vitest";
import type { DownstreamService, ProviderConfig } from "@/types/api";
import { ProviderServices } from "./provider-services";

const { mutate, rows } = vi.hoisted(() => ({
  mutate: vi.fn(),
  rows: [] as DownstreamService[],
}));
vi.mock("@/hooks/use-services", () => ({
  useProviderServices: () => ({ data: rows }),
  useServices: () => ({ data: rows }),
  useLinkProviderService: () => ({ mutate, isPending: false }),
}));
vi.mock("@tanstack/react-router", () => ({
  Link: ({
    params,
    children,
  }: {
    params: { serviceId: string };
    children: React.ReactNode;
  }) => <a href={`/services/${params.serviceId}/edit`}>{children}</a>,
}));
vi.mock("@/components/services/create-service-dialog", () => ({
  CreateServiceDialog: () => null,
}));

it("shows every linked service and routes configuration to its existing service record", async () => {
  rows.splice(
    0,
    rows.length,
    {
      id: "service-a",
      name: "Telnyx AI",
      slug: "api-telnyx",
      provider_config_id: "telnyx",
      credential_configured: true,
      legacy_public_master: false,
      platform_key: {
        enabled: true,
        audience: "restricted",
        allowed_owner_ids: ["owner"],
      },
    } as DownstreamService,
    {
      id: "service-b",
      name: "Calls",
      slug: "calls",
      provider_config_id: null,
    } as DownstreamService,
  );
  render(
    <ProviderServices
      provider={
        {
          id: "telnyx",
          name: "Telnyx",
          provider_type: "api_key",
          is_active: true,
        } as ProviderConfig
      }
    />,
  );
  expect(
    screen
      .getAllByRole("link", { name: "Configure service" })
      .map((link) => link.getAttribute("href")),
  ).toEqual(["/services/service-a/edit", "/services/service-b/edit"]);
  expect(screen.getByText(/Shared credential: configured/)).toHaveTextContent(
    "1 selected owners",
  );
  expect(screen.getByText(/Legacy provider requirement/)).toBeInTheDocument();
  await userEvent.click(
    screen.getByRole("button", { name: "Set primary provider" }),
  );
  expect(mutate).toHaveBeenCalledWith({
    providerId: "telnyx",
    serviceId: "service-b",
  });
});

it.each([
  [undefined, "status unavailable"],
  [null, "stored but unavailable"],
  [false, "not configured"],
  [true, "configured"],
] as const)(
  "renders credential status %s without guessing",
  (status, label) => {
    rows.splice(0, rows.length, {
      id: "service",
      name: "Service",
      slug: "api-telnyx",
      provider_config_id: "telnyx",
      credential_configured: status,
    } as DownstreamService);
    render(
      <ProviderServices
        provider={
          {
            id: "telnyx",
            name: "Telnyx",
            provider_type: "api_key",
            is_active: true,
          } as ProviderConfig
        }
      />,
    );
    expect(
      screen.getByText(new RegExp(`Shared credential: ${label}`)),
    ).toBeInTheDocument();
  },
);
