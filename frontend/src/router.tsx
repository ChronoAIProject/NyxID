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
  LandingPage,
  AiSetupPage,
  LoginPage,
  RegisterPage,
  DashboardPage,
  ApiKeyDetailPage,
  ServicesPage,
  ServiceListPage,
  ServiceDetailPage,
  ServiceEditPage,
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
  AdminAuditLogPage,
  AdminInviteCodesPage,
  CliAuthPage,
  SshTerminalPage,
  KeysPage,
  KeyDetailPage,
  ChannelBotsPage,
  ChannelBotDetailPage,
  ChannelConversationDetailPage,
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
      throw redirect({ to: "/dashboard" });
    }
  },
  component: AuthLayout,
});

const loginRoute = createRoute({
  path: "/login",
  getParentRoute: () => authLayout,
  validateSearch: (
    search: Record<string, unknown>,
  ): { return_to?: string } => ({
    ...(typeof search.return_to === "string"
      ? { return_to: search.return_to }
      : {}),
  }),
  component: LoginPage,
});

const registerRoute = createRoute({
  path: "/register",
  getParentRoute: () => authLayout,
  validateSearch: (
    search: Record<string, unknown>,
  ): { return_to?: string } => ({
    ...(typeof search.return_to === "string"
      ? { return_to: search.return_to }
      : {}),
  }),
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

const sshTerminalRoute = createRoute({
  path: "/ssh/$serviceId/terminal",
  getParentRoute: () => rootRoute,
  beforeLoad: () => {
    const { isAuthenticated, isLoading } = useAuthStore.getState();
    if (!isAuthenticated && !isLoading) {
      throw redirect({ to: "/login" });
    }
  },
  component: SshTerminalPage,
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

const landingRoute = createRoute({
  path: "/",
  getParentRoute: () => rootRoute,
  beforeLoad: () => {
    const { isAuthenticated, isLoading } = useAuthStore.getState();
    if (isAuthenticated && !isLoading) {
      throw redirect({ to: "/dashboard" });
    }
  },
  component: LandingPage,
});

const dashboardIndexRoute = createRoute({
  path: "/dashboard",
  getParentRoute: () => dashboardLayout,
  component: DashboardPage,
});

const apiKeysRedirectRoute = createRoute({
  path: "/api-keys",
  getParentRoute: () => dashboardLayout,
  beforeLoad: () => {
    throw redirect({ to: "/keys", search: { tab: "nyxid" } });
  },
  component: () => null,
});

// -- Redirect old paths --

const servicesRedirectRoute = createRoute({
  path: "/services",
  getParentRoute: () => dashboardLayout,
  beforeLoad: () => {
    const { user } = useAuthStore.getState();
    if (user?.is_admin) {
      // Admin users can still access the services management pages
      return;
    }
    throw redirect({ to: "/keys", search: {} });
  },
  component: ServicesPage,
});

const servicesIndexRoute = createRoute({
  path: "/",
  getParentRoute: () => servicesRedirectRoute,
  component: ServiceListPage,
});

const serviceDetailRoute = createRoute({
  path: "$serviceId",
  getParentRoute: () => servicesRedirectRoute,
  component: ServiceDetailPage,
});

const serviceEditRoute = createRoute({
  path: "$serviceId/edit",
  getParentRoute: () => servicesRedirectRoute,
  component: ServiceEditPage,
});

const connectionsRedirectRoute = createRoute({
  path: "/connections",
  getParentRoute: () => dashboardLayout,
  beforeLoad: () => {
    throw redirect({ to: "/keys", search: {} });
  },
  component: () => null,
});

const providersRedirectRoute = createRoute({
  path: "/providers",
  getParentRoute: () => dashboardLayout,
  beforeLoad: () => {
    const { user } = useAuthStore.getState();
    if (user?.is_admin) {
      // Admin users can still access the providers management pages
      return;
    }
    throw redirect({ to: "/keys", search: {} });
  },
  component: ProvidersLayout,
});

const providersIndexRoute = createRoute({
  path: "/",
  getParentRoute: () => providersRedirectRoute,
  component: ProvidersPage,
});

const providersCallbackRoute = createRoute({
  path: "callback",
  getParentRoute: () => providersRedirectRoute,
  component: ProvidersCallbackPage,
});

const providerManageRoute = createRoute({
  path: "manage",
  getParentRoute: () => providersRedirectRoute,
  component: ProviderListPage,
});

const providerDetailRoute = createRoute({
  path: "$providerId",
  getParentRoute: () => providersRedirectRoute,
  component: ProviderDetailPage,
});

const providerEditRoute = createRoute({
  path: "$providerId/edit",
  getParentRoute: () => providersRedirectRoute,
  component: ProviderEditPage,
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

const aiSetupRoute = createRoute({
  path: "/ai-setup",
  getParentRoute: () => dashboardLayout,
  component: AiSetupPage,
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

const keysRoute = createRoute({
  path: "/keys",
  getParentRoute: () => dashboardLayout,
  validateSearch: (search: Record<string, unknown>): { tab?: string } => ({
    ...(typeof search.tab === "string" ? { tab: search.tab } : {}),
  }),
  component: KeysPage,
});

const keyDetailRoute = createRoute({
  path: "/keys/$keyId",
  getParentRoute: () => dashboardLayout,
  component: KeyDetailPage,
});

const apiKeyDetailRoute = createRoute({
  path: "/keys/api-key/$keyId",
  getParentRoute: () => dashboardLayout,
  component: ApiKeyDetailPage,
});

const channelBotsRoute = createRoute({
  path: "/channel-bots",
  getParentRoute: () => dashboardLayout,
  component: ChannelBotsPage,
});

const channelBotDetailRoute = createRoute({
  path: "/channel-bots/$botId",
  getParentRoute: () => dashboardLayout,
  component: ChannelBotDetailPage,
});

const channelConversationDetailRoute = createRoute({
  path: "/channel-bots/$botId/conversations/$conversationId",
  getParentRoute: () => dashboardLayout,
  component: ChannelConversationDetailPage,
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
      throw redirect({ to: "/dashboard" });
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

const adminAuditLogRoute = createRoute({
  path: "audit-log",
  getParentRoute: () => adminLayout,
  component: AdminAuditLogPage,
});

const adminInviteCodesRoute = createRoute({
  path: "invite-codes",
  getParentRoute: () => adminLayout,
  component: AdminInviteCodesPage,
});

const routeTree = rootRoute.addChildren([
  landingRoute,
  authLayout.addChildren([loginRoute, registerRoute]),
  oauthConsentRoute,
  oauthErrorRoute,
  privacyRoute,
  cliAuthRoute,
  sshTerminalRoute,
  dashboardLayout.addChildren([
    dashboardIndexRoute,
    apiKeysRedirectRoute,
    servicesRedirectRoute.addChildren([
      servicesIndexRoute,
      serviceDetailRoute,
      serviceEditRoute,
    ]),
    connectionsRedirectRoute,
    providersRedirectRoute.addChildren([
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
    aiSetupRoute,
    notificationSettingsRoute,
    approvalHistoryRoute,
    approvalGrantsRoute,
    keysRoute,
    keyDetailRoute,
    apiKeyDetailRoute,
    nodesRoute,
    nodeDetailRoute,
    channelBotsRoute,
    channelBotDetailRoute,
    channelConversationDetailRoute,
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
    adminAuditLogRoute,
    adminInviteCodesRoute,
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
