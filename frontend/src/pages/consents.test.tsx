import { cleanup, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import { ConsentsPage } from "./consents";

const deletedId = "79557a31-84f7-444a-b9f8-75be2f5baa5a";
const fixtures = vi.hoisted(() => ({ viewMode: "grid", services: true }));

vi.mock("@tanstack/react-router", () => ({
  useRouterState: () => ({ tab: "apps" }),
  useNavigate: () => vi.fn(),
}));
vi.mock("@/components/shared/view-toggle", () => ({
  useViewMode: () => [fixtures.viewMode, vi.fn()],
  ViewToggle: () => null,
}));
vi.mock("@/hooks/use-consents", () => ({
  useMyConsents: () => ({
    data: { consents: [{
      id: "consent",
      client_id: "client",
      client_name: "Example app",
      scopes: "openid proxy",
      granted_at: "2026-11-11T00:00:00Z",
      expires_at: null,
      allowed_service_ids: ["active", deletedId, "paused", "authorized-id", "unnamed-id"],
      // Older replicas used the identifier as the placeholder's label.
      allowed_services: fixtures.services ? [
        { id: "active", label: "Example service", deleted: false },
        { id: deletedId, label: deletedId, deleted: true },
        { id: "paused", label: "Paused shared service", deleted: false, disabled: true },
        { id: "authorized-id", label: "owned-service-slug", deleted: false },
        { id: "unnamed-id", label: "—", deleted: false },
      ] : undefined,
    }] },
  }),
  useRevokeConsent: () => ({ mutateAsync: vi.fn() }),
}));
vi.mock("@/components/shared/machine-grant-card", () => ({
  MachineGrantCard: () => null,
}));

afterEach(cleanup);

it.each(["grid", "table"])("qualifies base consent and masks deleted services in the %s view and responsive cards", (mode) => {
  fixtures.viewMode = mode;
  const { container } = render(<ConsentsPage />);
  const copies = mode === "grid" ? 1 : 2;
  expect(screen.getAllByText("App consent granted")).toHaveLength(copies);
  expect(screen.getAllByText("App consent expiry")).toHaveLength(copies);
  expect(screen.getAllByText("Never")).toHaveLength(copies);
  expect(screen.getAllByText("Example service")).toHaveLength(copies);
  expect(screen.getAllByText("Deleted service")).toHaveLength(copies);
  expect(screen.getAllByText("Paused shared service (disabled)")).toHaveLength(copies);
  expect(screen.getAllByText("owned-service-slug")).toHaveLength(copies);
  expect(screen.getAllByText("Service details unavailable")).toHaveLength(copies);
  expect(container.innerHTML).not.toContain("unnamed-id");
  expect(container.innerHTML).not.toContain(deletedId);
});

it("shows when an older list response omits service details", () => {
  fixtures.viewMode = "grid";
  fixtures.services = false;
  render(<ConsentsPage />);
  expect(screen.getByText("Service details unavailable")).toBeInTheDocument();
});
