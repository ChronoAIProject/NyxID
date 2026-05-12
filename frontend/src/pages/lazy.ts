import { lazy } from "react";

export const LandingPage = lazy(() =>
  import("@/pages/landing").then((m) => ({ default: m.LandingPage })),
);
export const AiSetupPage = lazy(() =>
  import("@/pages/ai-setup").then((m) => ({ default: m.AiSetupPage })),
);
export const LoginPage = lazy(() =>
  import("@/pages/login").then((m) => ({ default: m.LoginPage })),
);
export const RegisterPage = lazy(() =>
  import("@/pages/register").then((m) => ({ default: m.RegisterPage })),
);
export const CliAuthPage = lazy(() =>
  import("@/pages/cli-auth").then((m) => ({ default: m.CliAuthPage })),
);
export const CliPairPage = lazy(() =>
  import("@/pages/cli-pair").then((m) => ({ default: m.CliPairPage })),
);
export const DashboardPage = lazy(() =>
  import("@/pages/dashboard").then((m) => ({ default: m.DashboardPage })),
);
export const ApiKeyDetailPage = lazy(() =>
  import("@/pages/api-key-detail").then((m) => ({
    default: m.ApiKeyDetailPage,
  })),
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
export const AuthorizationsPage = lazy(() =>
  import("@/pages/authorizations").then((m) => ({
    default: m.AuthorizationsPage,
  })),
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
export const PrivacyPage = lazy(() =>
  import("@/pages/privacy").then((m) => ({ default: m.PrivacyPage })),
);
export const TermsPage = lazy(() =>
  import("@/pages/terms").then((m) => ({ default: m.TermsPage })),
);
export const NotificationSettingsPage = lazy(() =>
  import("@/pages/notification-settings").then((m) => ({
    default: m.NotificationSettingsPage,
  })),
);
export const ApprovalHistoryPage = lazy(() =>
  import("@/pages/approval-history").then((m) => ({
    default: m.ApprovalHistoryPage,
  })),
);
export const ApprovalGrantsPage = lazy(() =>
  import("@/pages/approval-grants").then((m) => ({
    default: m.ApprovalGrantsPage,
  })),
);
export const NodesPage = lazy(() =>
  import("@/pages/nodes").then((m) => ({ default: m.NodesPage })),
);
export const NodeDetailPage = lazy(() =>
  import("@/pages/node-detail").then((m) => ({
    default: m.NodeDetailPage,
  })),
);
export const AdminNodesPage = lazy(() =>
  import("@/pages/admin-nodes").then((m) => ({
    default: m.AdminNodesPage,
  })),
);
export const AdminAuditLogPage = lazy(() =>
  import("@/pages/admin-audit-log").then((m) => ({
    default: m.AdminAuditLogPage,
  })),
);
export const AdminInviteCodesPage = lazy(() =>
  import("@/pages/admin-invite-codes").then((m) => ({
    default: m.AdminInviteCodesPage,
  })),
);
export const SshTerminalPage = lazy(() =>
  import("@/pages/ssh-terminal").then((m) => ({
    default: m.SshTerminalPage,
  })),
);
export const KeysPage = lazy(() =>
  import("@/pages/keys").then((m) => ({ default: m.KeysPage })),
);
export const KeyDetailPage = lazy(() =>
  import("@/pages/key-detail").then((m) => ({ default: m.KeyDetailPage })),
);
export const ChannelBotsPage = lazy(() =>
  import("@/pages/channel-bots").then((m) => ({
    default: m.ChannelBotsPage,
  })),
);
export const ChannelBotDetailPage = lazy(() =>
  import("@/pages/channel-bot-detail").then((m) => ({
    default: m.ChannelBotDetailPage,
  })),
);
export const ChannelConversationDetailPage = lazy(() =>
  import("@/pages/channel-conversation-detail").then((m) => ({
    default: m.ChannelConversationDetailPage,
  })),
);
export const OrgsPage = lazy(() =>
  import("@/pages/orgs").then((m) => ({ default: m.OrgsPage })),
);
export const OrgDetailPage = lazy(() =>
  import("@/pages/org-detail").then((m) => ({ default: m.OrgDetailPage })),
);
export const OrgServiceAccountDetailPage = lazy(() =>
  import("@/pages/org-service-account-detail").then((m) => ({
    default: m.OrgServiceAccountDetailPage,
  })),
);
export const OrgDeveloperAppDetailPage = lazy(() =>
  import("@/pages/org-developer-app-detail").then((m) => ({
    default: m.OrgDeveloperAppDetailPage,
  })),
);
export const OrgJoinPage = lazy(() =>
  import("@/pages/org-join").then((m) => ({ default: m.OrgJoinPage })),
);
export const BlogIndexPage = lazy(() =>
  import("@/pages/blog-index").then((m) => ({ default: m.BlogIndexPage })),
);
export const BlogDetailPage = lazy(() =>
  import("@/pages/blog-detail").then((m) => ({ default: m.BlogDetailPage })),
);
export const BlogPreviewPage = lazy(() =>
  import("@/pages/blog-preview").then((m) => ({ default: m.BlogPreviewPage })),
);
export const DesignSystemPage = lazy(() =>
  import("@/pages/design-system").then((m) => ({ default: m.DesignSystemPage })),
);
