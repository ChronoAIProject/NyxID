import { memo } from "react";
import { Link } from "@tanstack/react-router";
import { useAuthStore } from "@/stores/auth-store";
import { useApiKeys } from "@/hooks/use-api-keys";
import { useServices, useConnections } from "@/hooks/use-services";
import { Skeleton } from "@/components/ui/skeleton";
import { Key, Server, Link2, ShieldCheck, ShieldOff } from "lucide-react";
import { cn } from "@/lib/utils";

interface StatItem {
  readonly title: string;
  readonly value: number | string;
  readonly description: string;
  readonly icon: React.ComponentType<{ className?: string }>;
  readonly loading: boolean;
  readonly valueColor?: string;
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
        ? "Account protected"
        : "Enable for better security",
      icon: user?.mfa_enabled ? ShieldCheck : ShieldOff,
      loading: false,
      valueColor: user?.mfa_enabled ? "text-success" : "text-destructive",
    },
  ];

  return (
    <div className="flex flex-col gap-12">
      <div className="flex flex-col gap-2">
        <h2 className="font-display text-5xl font-normal tracking-tight">
          Welcome back{user?.name ? `, ${user.name}` : ""}
        </h2>
        <p className="text-sm text-muted-foreground">
          Here is an overview of your NyxID account
        </p>
      </div>

      <div className="grid gap-5 sm:grid-cols-2 lg:grid-cols-4">
        {stats.map((stat) => (
          <div
            key={stat.title}
            className="flex flex-col gap-4 rounded-[10px] border border-border bg-transparent p-6"
          >
            {/* Label + Icon row */}
            <div className="flex items-center justify-between">
              <span className="text-xs font-medium text-muted-foreground">
                {stat.title}
              </span>
              <stat.icon
                className="h-4 w-4 text-text-tertiary"
                aria-hidden="true"
              />
            </div>

            {/* Value */}
            {stat.loading ? (
              <Skeleton className="h-10 w-20" />
            ) : (
              <div className={cn("font-display text-[36px] font-normal leading-tight", stat.valueColor ?? "text-foreground")}>
                {stat.value}
              </div>
            )}

            {/* Description */}
            <span className="text-xs text-text-tertiary">
              {stat.description}
            </span>
          </div>
        ))}
      </div>

      <div className="grid gap-6 lg:grid-cols-2">
        <div className="flex flex-col gap-6 rounded-[10px] border border-border bg-transparent p-7">
          {/* Title */}
          <div className="flex flex-col gap-1">
            <h3 className="font-display text-[22px] font-normal">Quick Actions</h3>
            <p className="text-[13px] text-muted-foreground">Common tasks and shortcuts</p>
          </div>

          {/* List */}
          <div className="flex flex-col">
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
              isLast
            />
          </div>
        </div>

        <div className="flex flex-col gap-6 rounded-[10px] border border-border bg-transparent p-7">
          {/* Title */}
          <div className="flex flex-col gap-1">
            <h3 className="font-display text-[22px] font-normal">Account Info</h3>
            <p className="text-[13px] text-muted-foreground">Your account details</p>
          </div>

          {/* List */}
          <div className="flex flex-col gap-4">
            <InfoRow label="Email" value={user?.email ?? "N/A"} />
            <InfoRow
              label="Email verified"
              value={user?.email_verified ? "Yes" : "No"}
              valueColor={user?.email_verified ? "text-success" : undefined}
            />
            <InfoRow
              label="MFA"
              value={user?.mfa_enabled ? "Enabled" : "Disabled"}
              valueColor={user?.mfa_enabled ? "text-success" : undefined}
            />
            <InfoRow
              label="Member since"
              value={
                user?.created_at
                  ? new Date(user.created_at).toLocaleDateString("en-US", { month: "short", day: "numeric", year: "numeric" })
                  : "N/A"
              }
            />
          </div>
        </div>
      </div>
    </div>
  );
}

const QuickAction = memo(function QuickAction({
  label,
  to,
  icon,
  isLast = false,
}: {
  readonly label: string;
  readonly to: string;
  readonly icon: React.ReactNode;
  readonly isLast?: boolean;
}) {
  return (
    <Link
      to={to}
      className={cn(
        "flex items-center gap-[14px] py-3.5 text-[13px] text-foreground transition-colors hover:text-primary",
        !isLast && "border-b border-border",
      )}
    >
      <div className="text-text-tertiary">{icon}</div>
      <span>{label}</span>
    </Link>
  );
});

const InfoRow = memo(function InfoRow({
  label,
  value,
  valueColor,
}: {
  readonly label: string;
  readonly value: string;
  readonly valueColor?: string;
}) {
  return (
    <div className="flex items-center justify-between">
      <span className="text-[13px] text-muted-foreground">{label}</span>
      <span className={cn("text-[13px] font-medium", valueColor ?? "text-foreground")}>{value}</span>
    </div>
  );
});
