import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { afterEach, expect, it, vi } from "vitest";
import { useAuthStore } from "@/stores/auth-store";
import type { DownstreamService, User } from "@/types/api";
import { CatalogToolMetadata } from "./catalog-tool-metadata";

const mutation = vi.hoisted(() => vi.fn());
vi.mock("@/hooks/use-catalog-admin", () => ({
  useCatalogToolMutation: () => ({ mutateAsync: mutation, isPending: false }),
}));
vi.mock("@/hooks/use-tools", () => ({ useToolTopics: () => ({ data: [] }) }));
afterEach(() => {
  mutation.mockReset();
  useAuthStore.setState({ user: null });
});

it("submits safe capability and permission metadata without manager-forbidden shape fields", async () => {
  useAuthStore.setState({ user: { id: "manager", is_admin: false } as User });
  mutation.mockResolvedValue({});
  render(
    <CatalogToolMetadata
      service={
        {
          id: "own-tool",
          name: "Vendor",
          offering_kind: "tool",
          required_permissions: ["read:api"],
          slug: "vendor",
          description: null,
          base_url: "https://vendor.example/api",
          service_type: "http",
          visibility: "public",
          auth_method: "bearer",
          auth_type: null,
          auth_key_name: "Authorization",
          is_active: true,
          oauth_client_id: null,
          api_spec_url: null,
          service_category: "internal",
          requires_user_credential: false,
          created_by: "manager",
          created_at: "2026-10-13T00:00:00Z",
          updated_at: "2026-10-13T00:00:00Z",
          effective_platform_metric: "requests",
        } satisfies DownstreamService
      }
    />,
  );
  const save = screen.getByRole("button", { name: "Save metadata" });
  expect(save).toBeDisabled();
  expect(screen.queryByLabelText("Offering kind")).not.toBeInTheDocument();
  await userEvent.click(
    screen.getByText("Documentation and operational notes"),
  );
  await userEvent.click(screen.getByRole("switch", { name: "Streaming" }));
  expect(save).toBeEnabled();
  const permissions = screen.getByLabelText("Required permissions");
  await userEvent.clear(permissions);
  await userEvent.type(permissions, "read:api, write:data");
  await userEvent.click(save);
  await waitFor(() => expect(mutation).toHaveBeenCalledOnce());
  const request = mutation.mock.calls[0]![0];
  expect(request).toMatchObject({
    serviceId: "own-tool",
    body: {
      capabilities: { supports_streaming: true, supports_proxy_write: false },
      required_permissions: ["read:api", "write:data"],
    },
  });
  expect(request.body).not.toHaveProperty("offering_kind");
  expect(request.body).not.toHaveProperty("provider_config_id");
  expect(request.body).not.toHaveProperty("platform_key");
  await waitFor(() => expect(save).toBeDisabled());
});

it.each([false, true])("clears an uploaded icon for an owning manager or Admin (admin=%s)", async (isAdmin) => {
  useAuthStore.setState({ user: { id: "manager", is_admin: isAdmin } as User });
  mutation.mockResolvedValue({});
  render(<CatalogToolMetadata service={{ id: "tool", name: "Vendor", offering_kind: "tool", icon_url: "/catalog-icons/local" } as DownstreamService} />);
  await userEvent.click(screen.getByRole("button", { name: "Remove icon" }));
  await waitFor(() => expect(mutation).toHaveBeenCalledWith({ serviceId: "tool", body: { icon_url: "" } }));
});
