import { lazy } from "react";

export const LoginPage = lazy(() =>
  import("@/pages/login").then((m) => ({ default: m.LoginPage })),
);
export const RegisterPage = lazy(() =>
  import("@/pages/register").then((m) => ({ default: m.RegisterPage })),
);
export const DashboardPage = lazy(() =>
  import("@/pages/dashboard").then((m) => ({ default: m.DashboardPage })),
);
export const ApiKeysPage = lazy(() =>
  import("@/pages/api-keys").then((m) => ({ default: m.ApiKeysPage })),
);
export const ServicesPage = lazy(() =>
  import("@/pages/services").then((m) => ({ default: m.ServicesPage })),
);
export const ServiceListPage = lazy(() =>
  import("@/pages/service-list").then((m) => ({ default: m.ServiceListPage })),
);
export const ServiceDetailPage = lazy(() =>
  import("@/pages/service-detail").then((m) => ({
    default: m.ServiceDetailPage,
  })),
);
export const ServiceEditPage = lazy(() =>
  import("@/pages/service-edit").then((m) => ({
    default: m.ServiceEditPage,
  })),
);
export const ConnectionsPage = lazy(() =>
  import("@/pages/connections").then((m) => ({ default: m.ConnectionsPage })),
);
export const SettingsPage = lazy(() =>
  import("@/pages/settings").then((m) => ({ default: m.SettingsPage })),
);
export const GuidePage = lazy(() =>
  import("@/pages/guide").then((m) => ({ default: m.GuidePage })),
);
export const ProvidersLayout = lazy(() =>
  import("@/pages/providers-layout").then((m) => ({
    default: m.ProvidersLayout,
  })),
);
export const ProvidersPage = lazy(() =>
  import("@/pages/providers").then((m) => ({ default: m.ProvidersPage })),
);
export const ProvidersCallbackPage = lazy(() =>
  import("@/pages/providers-callback").then((m) => ({
    default: m.ProvidersCallbackPage,
  })),
);
export const ProviderListPage = lazy(() =>
  import("@/pages/provider-list").then((m) => ({
    default: m.ProviderListPage,
  })),
);
export const ProviderDetailPage = lazy(() =>
  import("@/pages/provider-detail").then((m) => ({
    default: m.ProviderDetailPage,
  })),
);
export const ProviderEditPage = lazy(() =>
  import("@/pages/provider-edit").then((m) => ({
    default: m.ProviderEditPage,
  })),
);
export const AdminUsersPage = lazy(() =>
  import("@/pages/admin-users").then((m) => ({ default: m.AdminUsersPage })),
);
export const AdminUserDetailPage = lazy(() =>
  import("@/pages/admin-user-detail").then((m) => ({
    default: m.AdminUserDetailPage,
  })),
);
export const AdminRolesPage = lazy(() =>
  import("@/pages/admin-roles").then((m) => ({ default: m.AdminRolesPage })),
);
export const AdminRoleDetailPage = lazy(() =>
  import("@/pages/admin-role-detail").then((m) => ({
    default: m.AdminRoleDetailPage,
  })),
);
export const AdminGroupsPage = lazy(() =>
  import("@/pages/admin-groups").then((m) => ({ default: m.AdminGroupsPage })),
);
export const AdminGroupDetailPage = lazy(() =>
  import("@/pages/admin-group-detail").then((m) => ({
    default: m.AdminGroupDetailPage,
  })),
);
export const AdminServiceAccountsPage = lazy(() =>
  import("@/pages/admin-service-accounts").then((m) => ({
    default: m.AdminServiceAccountsPage,
  })),
);
export const AdminServiceAccountDetailPage = lazy(() =>
  import("@/pages/admin-service-account-detail").then((m) => ({
    default: m.AdminServiceAccountDetailPage,
  })),
);
export const ConsentsPage = lazy(() =>
  import("@/pages/consents").then((m) => ({ default: m.ConsentsPage })),
);
export const DeveloperAppsPage = lazy(() =>
  import("@/pages/developer-apps").then((m) => ({
    default: m.DeveloperAppsPage,
  })),
);
export const DeveloperAppDetailPage = lazy(() =>
  import("@/pages/developer-app-detail").then((m) => ({
    default: m.DeveloperAppDetailPage,
  })),
);
export const IntegrationGuidePage = lazy(() =>
  import("@/pages/integration-guide").then((m) => ({
    default: m.IntegrationGuidePage,
  })),
);
export const OAuthConsentPage = lazy(() =>
  import("@/pages/oauth-consent").then((m) => ({
    default: m.OAuthConsentPage,
  })),
);
export const OAuthErrorPage = lazy(() =>
  import("@/pages/oauth-error").then((m) => ({ default: m.OAuthErrorPage })),
);
