import { cleanup, fireEvent, render, screen } from "@testing-library/react";
import { afterEach, expect, it, vi } from "vitest";
import type { ReactNode } from "react";
import { ApiKeyTable } from "./api-key-table";

vi.mock("@tanstack/react-router", () => ({
  useNavigate: () => vi.fn(),
  Link: ({ children }: { children: ReactNode }) => <span>{children}</span>,
}));
vi.mock("@/hooks/use-api-keys", () => ({
  useAllAdminedApiKeys: () => ({
    data: [
      {
        id: "ordinary",
        name: "CI agent",
        platform: "codex",
        key_prefix: "nyxid_ag_123",
        scopes: "proxy",
        is_active: true,
        bindings_count: 0,
        created_at: "2026-09-17T00:00:00Z",
      },
      {
        id: "chat",
        name: "NyxID Assistant chat 12345678",
        platform: "nyxid-assistant",
        key_prefix: "nyxid_ag_456",
        scopes: "proxy",
        is_active: true,
        bindings_count: 0,
        created_at: "2026-09-17T00:00:00Z",
      },
    ],
  }),
  useDeleteApiKey: () => ({ mutateAsync: vi.fn() }),
  useRotateApiKey: () => ({ mutateAsync: vi.fn() }),
}));
afterEach(cleanup);

it.each(["table", "grid"] as const)(
  "hides assistant chat keys by default in %s view",
  (viewMode) => {
    render(<ApiKeyTable viewMode={viewMode} />);
    expect(screen.getAllByText("CI agent").length).toBeGreaterThan(0);
    expect(screen.queryByText("NyxID Assistant chat 12345678")).not.toBeInTheDocument();
    fireEvent.click(screen.getByRole("switch", { name: "Show assistant chat keys" }));
    expect(screen.getAllByText("NyxID Assistant chat 12345678").length).toBeGreaterThan(0);
    fireEvent.click(screen.getByRole("switch", { name: "Show assistant chat keys" }));
    expect(screen.queryByText("NyxID Assistant chat 12345678")).not.toBeInTheDocument();
  },
);
