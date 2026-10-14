import { afterEach, expect, it, vi } from "vitest";
import { render, screen } from "@testing-library/react";
import { QueryClient, QueryClientProvider } from "@tanstack/react-query";
import { Sidebar } from "./sidebar";
import { useAuthStore } from "@/stores/auth-store";
import type { User } from "@/types/api";

vi.mock("@tanstack/react-router", () => ({
  useRouterState: () => ({ location: { pathname: "/admin/feature-flags" } }),
  Link: ({ to, children }: { to: string; children: React.ReactNode }) => (
    <a href={to}>{children}</a>
  ),
}));

const initialAuth = useAuthStore.getState();
afterEach(() => useAuthStore.setState(initialAuth));

it("renders Feature Flags navigation for a regular user holding a feature flag role", () => {
  useAuthStore.setState({
    user: {
      id: "flag-operator",
      is_admin: false,
      role: "user",
      capabilities: { read_feature_flags: true, create_feature_flags: true },
    } as User,
  });
  const client = new QueryClient();
  render(
    <QueryClientProvider client={client}>
      <Sidebar mobile />
    </QueryClientProvider>,
  );
  expect(screen.getByRole("link", { name: "Feature Flags" })).toHaveAttribute(
    "href",
    "/admin/feature-flags",
  );
  expect(screen.queryByRole("link", { name: "Users" })).not.toBeInTheDocument();
  expect(screen.queryByRole("link", { name: "Roles" })).not.toBeInTheDocument();
});
