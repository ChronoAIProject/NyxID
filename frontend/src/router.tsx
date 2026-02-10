import {
  createRouter,
  createRoute,
  createRootRoute,
  redirect,
  Outlet,
} from "@tanstack/react-router";
import { TooltipProvider } from "@/components/ui/tooltip";
import { Toaster } from "@/components/ui/toast";
import { AuthLayout } from "@/components/layout/auth-layout";
import { DashboardLayout } from "@/components/layout/dashboard-layout";
import { useAuthStore } from "@/stores/auth-store";

import { LoginPage } from "@/pages/login";
import { RegisterPage } from "@/pages/register";
import { DashboardPage } from "@/pages/dashboard";
import { ApiKeysPage } from "@/pages/api-keys";
import { ServicesPage } from "@/pages/services";
import { ServiceListPage } from "@/pages/service-list";
import { ServiceDetailPage } from "@/pages/service-detail";
import { ServiceEditPage } from "@/pages/service-edit";
import { ConnectionsPage } from "@/pages/connections";
import { SettingsPage } from "@/pages/settings";
import { GuidePage } from "@/pages/guide";

const rootRoute = createRootRoute({
  component: () => (
    <TooltipProvider>
      <Outlet />
      <Toaster />
    </TooltipProvider>
  ),
});

const authLayout = createRoute({
  id: "auth",
  getParentRoute: () => rootRoute,
  beforeLoad: () => {
    const { isAuthenticated, isLoading } = useAuthStore.getState();
    if (isAuthenticated && !isLoading) {
      throw redirect({ to: "/" });
    }
  },
  component: AuthLayout,
});

const loginRoute = createRoute({
  path: "/login",
  getParentRoute: () => authLayout,
  component: LoginPage,
});

const registerRoute = createRoute({
  path: "/register",
  getParentRoute: () => authLayout,
  component: RegisterPage,
});

const dashboardLayout = createRoute({
  id: "dashboard",
  getParentRoute: () => rootRoute,
  beforeLoad: () => {
    const { isAuthenticated, isLoading } = useAuthStore.getState();
    if (!isAuthenticated && !isLoading) {
      throw redirect({ to: "/login" });
    }
  },
  component: DashboardLayout,
});

const dashboardIndexRoute = createRoute({
  path: "/",
  getParentRoute: () => dashboardLayout,
  component: DashboardPage,
});

const apiKeysRoute = createRoute({
  path: "/api-keys",
  getParentRoute: () => dashboardLayout,
  component: ApiKeysPage,
});

const servicesLayout = createRoute({
  path: "/services",
  getParentRoute: () => dashboardLayout,
  component: ServicesPage,
});

const servicesIndexRoute = createRoute({
  path: "/",
  getParentRoute: () => servicesLayout,
  component: ServiceListPage,
});

const serviceDetailRoute = createRoute({
  path: "$serviceId",
  getParentRoute: () => servicesLayout,
  component: ServiceDetailPage,
});

const serviceEditRoute = createRoute({
  path: "$serviceId/edit",
  getParentRoute: () => servicesLayout,
  component: ServiceEditPage,
});

const connectionsRoute = createRoute({
  path: "/connections",
  getParentRoute: () => dashboardLayout,
  component: ConnectionsPage,
});

const settingsRoute = createRoute({
  path: "/settings",
  getParentRoute: () => dashboardLayout,
  component: SettingsPage,
});

const guideRoute = createRoute({
  path: "/guide",
  getParentRoute: () => dashboardLayout,
  component: GuidePage,
});

const routeTree = rootRoute.addChildren([
  authLayout.addChildren([loginRoute, registerRoute]),
  dashboardLayout.addChildren([
    dashboardIndexRoute,
    apiKeysRoute,
    servicesLayout.addChildren([
      servicesIndexRoute,
      serviceDetailRoute,
      serviceEditRoute,
    ]),
    connectionsRoute,
    settingsRoute,
    guideRoute,
  ]),
]);

export const router = createRouter({
  routeTree,
  defaultPreload: "intent",
});

declare module "@tanstack/react-router" {
  interface Register {
    router: typeof router;
  }
}
