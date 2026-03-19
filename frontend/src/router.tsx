import { Suspense } from "react";
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

import {
  LoginPage,
  RegisterPage,
  DashboardPage,
  ApiKeysPage,
  ServicesPage,
  ServiceListPage,
  ServiceDetailPage,
  ServiceEditPage,
  ConnectionsPage,
  SettingsPage,
  GuidePage,
  ProvidersLayout,
  ProvidersPage,
  ProvidersCallbackPage,
  ProviderListPage,
  ProviderDetailPage,
  ProviderEditPage,
  AdminUsersPage,
  AdminUserDetailPage,
  AdminRolesPage,
  AdminRoleDetailPage,
  AdminGroupsPage,
  AdminGroupDetailPage,
  AdminServiceAccountsPage,
  AdminServiceAccountDetailPage,
  ConsentsPage,
  DeveloperAppsPage,
  DeveloperAppDetailPage,
  IntegrationGuidePage,
  OAuthConsentPage,
  OAuthErrorPage,
  PrivacyPage,
  NotificationSettingsPage,
  ApprovalHistoryPage,
  ApprovalGrantsPage,
  NodesPage,
  NodeDetailPage,
  AdminNodesPage,
  CliAuthPage,
} from "@/pages/lazy";

// ── Route tree ──

const rootRoute = createRootRoute({
  component: () => (
    <TooltipProvider>
      <Suspense>
        <Outlet />
      </Suspense>
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
      const returnTo = new URLSearchParams(window.location.search).get(
        "return_to",
      );
      if (returnTo && returnTo.startsWith(window.location.origin + "/")) {
        window.location.assign(returnTo);
        return;
      }
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

const oauthConsentRoute = createRoute({
  path: "/oauth-consent",
  getParentRoute: () => rootRoute,
  beforeLoad: () => {
    const { isAuthenticated, isLoading } = useAuthStore.getState();
    if (!isAuthenticated && !isLoading) {
      const returnPath = `${window.location.pathname}${window.location.search}`;
      const returnTo = `${window.location.origin}${returnPath}`;
      window.location.assign(
        `/login?return_to=${encodeURIComponent(returnTo)}`,
      );
      return;
    }
  },
  component: OAuthConsentPage,
});

const oauthErrorRoute = createRoute({
  path: "/error",
  getParentRoute: () => rootRoute,
  component: OAuthErrorPage,
});

const privacyRoute = createRoute({
  path: "/privacy",
  getParentRoute: () => rootRoute,
  component: PrivacyPage,
});

const cliAuthRoute = createRoute({
  path: "/cli-auth",
  getParentRoute: () => rootRoute,
  component: CliAuthPage,
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

const consentsRoute = createRoute({
  path: "/settings/consents",
  getParentRoute: () => dashboardLayout,
  component: ConsentsPage,
});

const developerAppsRoute = createRoute({
  path: "/developer/apps",
  getParentRoute: () => dashboardLayout,
  component: DeveloperAppsPage,
});

const developerAppDetailRoute = createRoute({
  path: "/developer/apps/$clientId",
  getParentRoute: () => dashboardLayout,
  component: DeveloperAppDetailPage,
});

const integrationGuideRoute = createRoute({
  path: "/integration-guide",
  getParentRoute: () => dashboardLayout,
  component: IntegrationGuidePage,
});

const providersLayout = createRoute({
  path: "/providers",
  getParentRoute: () => dashboardLayout,
  component: ProvidersLayout,
});

const providersIndexRoute = createRoute({
  path: "/",
  getParentRoute: () => providersLayout,
  component: ProvidersPage,
});

const providersCallbackRoute = createRoute({
  path: "callback",
  getParentRoute: () => providersLayout,
  component: ProvidersCallbackPage,
});

const providerManageRoute = createRoute({
  path: "manage",
  getParentRoute: () => providersLayout,
  component: ProviderListPage,
});

const providerDetailRoute = createRoute({
  path: "$providerId",
  getParentRoute: () => providersLayout,
  component: ProviderDetailPage,
});

const providerEditRoute = createRoute({
  path: "$providerId/edit",
  getParentRoute: () => providersLayout,
  component: ProviderEditPage,
});

const notificationSettingsRoute = createRoute({
  path: "/approvals/settings",
  getParentRoute: () => dashboardLayout,
  component: NotificationSettingsPage,
});

const approvalHistoryRoute = createRoute({
  path: "/approvals/history",
  getParentRoute: () => dashboardLayout,
  component: ApprovalHistoryPage,
});

const approvalGrantsRoute = createRoute({
  path: "/approvals/grants",
  getParentRoute: () => dashboardLayout,
  component: ApprovalGrantsPage,
});

const nodesRoute = createRoute({
  path: "/nodes",
  getParentRoute: () => dashboardLayout,
  component: NodesPage,
});

const nodeDetailRoute = createRoute({
  path: "/nodes/$nodeId",
  getParentRoute: () => dashboardLayout,
  component: NodeDetailPage,
});

const adminLayout = createRoute({
  path: "/admin",
  getParentRoute: () => dashboardLayout,
  beforeLoad: () => {
    const { user, isAuthenticated, isLoading } = useAuthStore.getState();
    if (!isAuthenticated && !isLoading) {
      throw redirect({ to: "/login" });
    }
    if (!isLoading && (!user || !user.is_admin)) {
      throw redirect({ to: "/" });
    }
  },
  component: () => <Outlet />,
});

const adminUsersRoute = createRoute({
  path: "users",
  getParentRoute: () => adminLayout,
  component: AdminUsersPage,
});

const adminUserDetailRoute = createRoute({
  path: "users/$userId",
  getParentRoute: () => adminLayout,
  component: AdminUserDetailPage,
});

const adminRolesRoute = createRoute({
  path: "roles",
  getParentRoute: () => adminLayout,
  component: AdminRolesPage,
});

const adminRoleDetailRoute = createRoute({
  path: "roles/$roleId",
  getParentRoute: () => adminLayout,
  component: AdminRoleDetailPage,
});

const adminGroupsRoute = createRoute({
  path: "groups",
  getParentRoute: () => adminLayout,
  component: AdminGroupsPage,
});

const adminGroupDetailRoute = createRoute({
  path: "groups/$groupId",
  getParentRoute: () => adminLayout,
  component: AdminGroupDetailPage,
});

const adminServiceAccountsRoute = createRoute({
  path: "service-accounts",
  getParentRoute: () => adminLayout,
  component: AdminServiceAccountsPage,
});

const adminServiceAccountDetailRoute = createRoute({
  path: "service-accounts/$saId",
  getParentRoute: () => adminLayout,
  component: AdminServiceAccountDetailPage,
});

const adminNodesRoute = createRoute({
  path: "nodes",
  getParentRoute: () => adminLayout,
  component: AdminNodesPage,
});

const routeTree = rootRoute.addChildren([
  authLayout.addChildren([loginRoute, registerRoute]),
  oauthConsentRoute,
  oauthErrorRoute,
  privacyRoute,
  cliAuthRoute,
  dashboardLayout.addChildren([
    dashboardIndexRoute,
    apiKeysRoute,
    servicesLayout.addChildren([
      servicesIndexRoute,
      serviceDetailRoute,
      serviceEditRoute,
    ]),
    connectionsRoute,
    providersLayout.addChildren([
      providersIndexRoute,
      providersCallbackRoute,
      providerManageRoute,
      providerDetailRoute,
      providerEditRoute,
    ]),
    settingsRoute,
    consentsRoute,
    guideRoute,
    developerAppsRoute,
    developerAppDetailRoute,
    integrationGuideRoute,
    notificationSettingsRoute,
    approvalHistoryRoute,
    approvalGrantsRoute,
    nodesRoute,
    nodeDetailRoute,
    adminLayout.addChildren([
      adminUsersRoute,
      adminUserDetailRoute,
      adminRolesRoute,
      adminRoleDetailRoute,
      adminGroupsRoute,
      adminGroupDetailRoute,
      adminServiceAccountsRoute,
      adminServiceAccountDetailRoute,
      adminNodesRoute,
    ]),
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
