import { memo } from "react";
import { Link } from "@tanstack/react-router";
import { useAuthStore } from "@/stores/auth-store";
import { useApiKeys } from "@/hooks/use-api-keys";
import { useServices, useConnections } from "@/hooks/use-services";
import {
  Card,
  CardContent,
  CardDescription,
  CardHeader,
  CardTitle,
} from "@/components/ui/card";
import { Skeleton } from "@/components/ui/skeleton";
import { Key, Server, Link2, ShieldCheck, ShieldOff } from "lucide-react";

interface StatItem {
  readonly title: string;
  readonly value: number | string;
  readonly description: string;
  readonly icon: React.ComponentType<{ className?: string }>;
  readonly loading: boolean;
}

export function DashboardPage() {
  const user = useAuthStore((s) => s.user);
  const { data: apiKeys, isLoading: keysLoading } = useApiKeys();
  const { data: services, isLoading: servicesLoading } = useServices();
  const { data: connections, isLoading: connectionsLoading } = useConnections();

  const stats: readonly StatItem[] = [
    {
      title: "API Keys",
      value: apiKeys?.filter((k) => !k.revoked).length ?? 0,
      description: "Active keys",
      icon: Key,
      loading: keysLoading,
    },
    {
      title: "Services",
      value: services?.length ?? 0,
      description: "Registered services",
      icon: Server,
      loading: servicesLoading,
    },
    {
      title: "Connections",
      value: connections?.length ?? 0,
      description: "Active connections",
      icon: Link2,
      loading: connectionsLoading,
    },
    {
      title: "MFA Status",
      value: user?.mfa_enabled ? "Enabled" : "Disabled",
      description: user?.mfa_enabled
        ? "Account is protected"
        : "Enable for better security",
      icon: user?.mfa_enabled ? ShieldCheck : ShieldOff,
      loading: false,
    },
  ];

  return (
    <div className="space-y-8">
      <div>
        <h2 className="text-3xl font-bold tracking-tight">
          Welcome back{user?.name ? `, ${user.name}` : ""}
        </h2>
        <p className="text-muted-foreground">
          Here is an overview of your NyxID account.
        </p>
      </div>

      <div className="grid gap-4 sm:grid-cols-2 lg:grid-cols-4">
        {stats.map((stat) => (
          <Card key={stat.title}>
            <CardHeader className="flex flex-row items-center justify-between space-y-0 pb-2">
              <CardTitle className="text-sm font-medium">
                {stat.title}
              </CardTitle>
              <stat.icon
                className="h-4 w-4 text-muted-foreground"
                aria-hidden="true"
              />
            </CardHeader>
            <CardContent>
              {stat.loading ? (
                <Skeleton className="h-8 w-16" />
              ) : (
                <div className="text-2xl font-bold">{stat.value}</div>
              )}
              <CardDescription className="text-xs">
                {stat.description}
              </CardDescription>
            </CardContent>
          </Card>
        ))}
      </div>

      <div className="grid gap-4 lg:grid-cols-2">
        <Card>
          <CardHeader>
            <CardTitle>Quick Actions</CardTitle>
            <CardDescription>Common tasks and shortcuts</CardDescription>
          </CardHeader>
          <CardContent className="space-y-2">
            <QuickAction
              label="Create a new API key"
              to="/api-keys"
              icon={<Key className="h-4 w-4" aria-hidden="true" />}
            />
            <QuickAction
              label="Register a service"
              to="/services"
              icon={<Server className="h-4 w-4" aria-hidden="true" />}
            />
            <QuickAction
              label="Manage connections"
              to="/connections"
              icon={<Link2 className="h-4 w-4" aria-hidden="true" />}
            />
          </CardContent>
        </Card>

        <Card>
          <CardHeader>
            <CardTitle>Account Info</CardTitle>
            <CardDescription>Your account details</CardDescription>
          </CardHeader>
          <CardContent className="space-y-3">
            <InfoRow label="Email" value={user?.email ?? "N/A"} />
            <InfoRow
              label="Email verified"
              value={user?.email_verified ? "Yes" : "No"}
            />
            <InfoRow
              label="MFA"
              value={user?.mfa_enabled ? "Enabled" : "Disabled"}
            />
            <InfoRow
              label="Member since"
              value={
                user?.created_at
                  ? new Date(user.created_at).toLocaleDateString()
                  : "N/A"
              }
            />
          </CardContent>
        </Card>
      </div>
    </div>
  );
}

const QuickAction = memo(function QuickAction({
  label,
  to,
  icon,
}: {
  readonly label: string;
  readonly to: string;
  readonly icon: React.ReactNode;
}) {
  return (
    <Link
      to={to}
      className="flex items-center gap-3 rounded-lg border p-3 text-sm transition-colors hover:bg-accent"
    >
      <div className="text-muted-foreground">{icon}</div>
      <span>{label}</span>
    </Link>
  );
});

const InfoRow = memo(function InfoRow({
  label,
  value,
}: {
  readonly label: string;
  readonly value: string;
}) {
  return (
    <div className="flex items-center justify-between">
      <span className="text-sm text-muted-foreground">{label}</span>
      <span className="text-sm font-medium">{value}</span>
    </div>
  );
});
