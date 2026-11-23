import { render, screen } from "@testing-library/react";
import userEvent from "@testing-library/user-event";
import { beforeEach, expect, it, vi } from "vitest";
import type { CatalogEntry, KeyInfo } from "@/types/keys";
const state = vi.hoisted(() => ({
  rows: [] as KeyInfo[],
  connect: vi.fn(),
  update: vi.fn(),
  hosted: vi.fn(),
  ready: true,
  retryCapabilities: vi.fn(),
}));
vi.mock("@/hooks/use-approvals", () => ({
  useApprovalAuthoringCapabilities: () => ({
    data: { catalog_opt_in: state.ready },
    refetch: state.retryCapabilities,
  }),
}));
vi.mock("@/hooks/use-keys", () => ({
  useCatalogEntry: () => ({}),
  useKeys: () => ({ data: state.rows, isLoading: false }),
  useConnectCatalogService: () => ({
    mutateAsync: state.connect,
    isPending: false,
  }),
  useUpdateKey: () => ({ mutateAsync: state.update, isPending: false }),
}));
vi.mock("@/hooks/use-connect-links", () => ({
  useCreateAssistantConnectLink: () => ({
    mutateAsync: state.hosted,
    isPending: false,
  }),
}));
import { CatalogConnectionAction } from "./catalog-connection-action";
const entry = {
  slug: "weather",
  name: "Weather",
  requires_credential: false,
  requires_gateway_url: false,
} as CatalogEntry;
beforeEach(() => {
  state.rows = [];
  state.ready = true;
  state.retryCapabilities.mockReset();
  state.connect
    .mockReset()
    .mockResolvedValue({ connection_id: "c", is_active: true });
  state.update.mockReset().mockResolvedValue({});
  state.hosted.mockReset();
});
it("connects the no-row service explicitly through the shared catalog API", async () => {
  render(<CatalogConnectionAction slug="weather" catalogEntry={entry} />);
  await userEvent.click(screen.getByRole("button", { name: "Connect" }));
  expect(state.connect).toHaveBeenCalledWith({ slug: "weather" });
  expect(state.hosted).not.toHaveBeenCalled();
});
it("adopts an automatic platform connection instead of calling credential setup", async () => {
  state.rows = [
    {
      id: "automatic",
      auto_connected: true,
      catalog_service_slug: "weather",
    } as KeyInfo,
  ];
  render(
    <CatalogConnectionAction
      slug="weather"
      catalogEntry={{
        ...entry,
        requires_credential: true,
        platform_key: { available: true },
      }}
    />,
  );
  await userEvent.click(screen.getByRole("button", { name: "Connect" }));
  expect(state.connect).toHaveBeenCalledWith({ slug: "weather" });
});
it("offers each explicit sibling separately and preserves the selected identity", async () => {
  state.rows = [
    {
      id: "a",
      label: "First",
      is_active: true,
      catalog_service_slug: "weather",
    },
    {
      id: "b",
      label: "Second",
      is_active: false,
      catalog_service_slug: "weather",
    },
  ] as KeyInfo[];
  render(<CatalogConnectionAction slug="weather" catalogEntry={entry} />);
  await userEvent.click(screen.getByRole("button", { name: "Enable" }));
  expect(state.update).toHaveBeenLastCalledWith({
    keyId: "b",
    is_active: true,
  });
  await userEvent.click(screen.getByRole("button", { name: "Disable" }));
  expect(state.update).toHaveBeenLastCalledWith({
    keyId: "a",
    is_active: false,
  });
  expect(state.connect).not.toHaveBeenCalled();
});
it("does not expose mutation controls for an inherited org connection", () => {
  state.rows = [
    {
      id: "org",
      label: "Org",
      is_active: true,
      catalog_service_slug: "weather",
      credential_source: { type: "org", role: "viewer" },
    } as KeyInfo,
  ];
  render(<CatalogConnectionAction slug="weather" catalogEntry={entry} />);
  expect(
    screen.queryByRole("button", { name: "Disable" }),
  ).not.toBeInTheDocument();
  expect(screen.getByRole("button", { name: "Connect" })).toBeInTheDocument();
});
it("does not submit new consent before all servers support opt-in", () => {
  state.ready = false;
  render(<CatalogConnectionAction slug="weather" catalogEntry={entry} />);
  expect(screen.getByRole("button", { name: "Connect" })).toBeDisabled();
});

it("keeps hosted credential setup enabled during mixed readiness", () => {
  state.ready = false;
  render(
    <CatalogConnectionAction
      slug="weather"
      catalogEntry={{ ...entry, requires_credential: true }}
    />,
  );
  expect(screen.getByRole("button", { name: "Connect" })).toBeEnabled();
});
it("offers a capability retry for an unavailable catalog opt-in", async () => {
  state.ready = false;
  render(<CatalogConnectionAction slug="weather" catalogEntry={entry} />);
  await userEvent.click(screen.getByRole("button", { name: "Retry" }));
  expect(state.retryCapabilities).toHaveBeenCalledOnce();
  expect(state.connect).not.toHaveBeenCalled();
});
