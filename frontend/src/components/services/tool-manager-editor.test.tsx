import { render, screen, waitFor } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import type { DownstreamService } from "@/types/api";
import { ToolManagerEditor } from "./tool-manager-editor";

const mocks = vi.hoisted(() => ({ update: vi.fn(), icon: vi.fn() }));
vi.mock("@/hooks/use-catalog-admin", () => ({
  useCatalogToolMutation: () => ({
    mutateAsync: mocks.update,
    isPending: false,
  }),
  useToolIcon: () => ({ mutate: mocks.icon, isPending: false }),
}));
vi.mock("./catalog-tool-metadata", () => ({
  CatalogToolMetadata: () => <div>Safe metadata</div>,
}));
vi.mock("@/components/dashboard/endpoint-list", () => ({
  EndpointList: () => <div>Operations</div>,
}));

const tool = {
  id: "own-tool",
  name: "Vendor",
  base_url: "https://vendor.example",
  auth_method: "bearer",
  credential_configured: false,
  tools_credential_frozen: false,
} as DownstreamService;
beforeEach(() => {
  mocks.update.mockReset();
  mocks.icon.mockReset();
  mocks.update.mockResolvedValue({});
});

it("installs only the first credential through a password field and clears it on success", async () => {
  render(<ToolManagerEditor service={tool} />);
  const field = screen.getByLabelText("Shared vendor credential");
  expect(field).toHaveAttribute("type", "password");
  await userEvent.type(field, "fake-local-value");
  await userEvent.click(
    screen.getByRole("button", { name: "Set first credential" }),
  );
  await waitFor(() =>
    expect(mocks.update).toHaveBeenCalledWith({
      serviceId: "own-tool",
      body: { credential: "fake-local-value" },
    }),
  );
  await waitFor(() => expect(field).toHaveValue(""));
});

it("keeps destination and credentials frozen after an Admin clears the credential", () => {
  render(
    <ToolManagerEditor service={{ ...tool, tools_credential_frozen: true }} />,
  );
  expect(
    screen.queryByLabelText("Shared vendor credential"),
  ).not.toBeInTheDocument();
  expect(
    screen.queryByRole("button", { name: "Save destination" }),
  ).not.toBeInTheDocument();
  expect(screen.getByLabelText("API base URL")).toBeDisabled();
  expect(
    screen.getByText("An Admin must restore the credential."),
  ).toBeInTheDocument();
});

it("skips credential entry for no-auth and uploads a Tool-owned SVG", async () => {
  render(
    <ToolManagerEditor
      service={{
        ...tool,
        auth_method: "none",
        icon_url: "/catalog-icons/local",
      }}
    />,
  );
  expect(
    screen.queryByLabelText("Shared vendor credential"),
  ).not.toBeInTheDocument();
  expect(screen.getByAltText("Current tool icon")).toHaveAttribute(
    "src",
    "/catalog-icons/local",
  );
  const file = new File(['<svg viewBox="0 0 24 24"/>'], "local.svg", {
    type: "image/svg+xml",
  });
  await userEvent.upload(screen.getByLabelText("Upload Tool SVG"), file);
  expect(mocks.icon).toHaveBeenCalledWith(
    { serviceId: "own-tool", file },
    expect.any(Object),
  );
});
