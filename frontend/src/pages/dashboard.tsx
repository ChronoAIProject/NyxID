import { memo, useState, useCallback } from "react";
import { Link } from "@tanstack/react-router";
import { useAuthStore } from "@/stores/auth-store";
import { useApiKeys } from "@/hooks/use-api-keys";
import { useServices, useConnections } from "@/hooks/use-services";
import { Skeleton } from "@/components/ui/skeleton";
import { Button } from "@/components/ui/button";
import { Key, Server, Link2, ShieldCheck, ShieldOff, Sparkles, X } from "lucide-react";
import { cn } from "@/lib/utils";
import { NotificationSetupCard } from "@/components/dashboard/notification-setup-card";

const AI_SETUP_DISMISSED_KEY = "nyxid:ai-setup-dismissed";

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
      value: apiKeys?.filter((k) => k.is_active).length ?? 0,
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

  const [aiSetupDismissed, setAiSetupDismissed] = useState(
    () => localStorage.getItem(AI_SETUP_DISMISSED_KEY) === "true",
  );

  const dismissAiSetup = useCallback(() => {
    localStorage.setItem(AI_SETUP_DISMISSED_KEY, "true");
    setAiSetupDismissed(true);
  }, []);

  return (
    <div className="flex flex-col gap-12">
      <div className="flex flex-col gap-2">
        <h2 className="font-display text-3xl font-normal tracking-tight md:text-5xl">
          Welcome back{user?.display_name ? `, ${user.display_name}` : ""}
        </h2>
        <p className="text-sm text-muted-foreground">
          Here is an overview of your NyxID account
        </p>
      </div>

      {!aiSetupDismissed && (
        <div className="relative flex items-start gap-4 rounded-[10px] border border-primary/20 bg-primary/5 p-5">
          <Sparkles className="mt-0.5 h-5 w-5 shrink-0 text-primary" aria-hidden="true" />
          <div className="flex flex-1 flex-col gap-1.5">
            <h3 className="text-sm font-medium text-foreground">
              Set up AI skills
            </h3>
            <p className="text-[13px] text-muted-foreground">
              Install NyxID skills in your AI agent (Claude Code, Cursor, Codex,
              OpenClaw) so it can help you manage services, credentials, and
              more -- no setup prompt needed each session.
            </p>
            <Link
              to="/ai-setup"
              className="mt-1 text-[13px] font-medium text-primary hover:underline"
            >
              Go to AI Setup
            </Link>
          </div>
          <Button
            variant="ghost"
            size="sm"
            className="absolute right-2 top-2 h-7 w-7 p-0 text-muted-foreground hover:text-foreground"
            onClick={dismissAiSetup}
            aria-label="Dismiss"
          >
            <X className="h-3.5 w-3.5" />
          </Button>
        </div>
      )}

      <NotificationSetupCard />

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
              <div
                className={cn(
                  "font-display text-[28px] font-normal leading-tight md:text-[36px]",
                  stat.valueColor ?? "text-foreground",
                )}
              >
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
            <h3 className="font-display text-[22px] font-normal">
              Quick Actions
            </h3>
            <p className="text-[13px] text-muted-foreground">
              Common tasks and shortcuts
            </p>
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
            <h3 className="font-display text-[22px] font-normal">
              Account Info
            </h3>
            <p className="text-[13px] text-muted-foreground">
              Your account details
            </p>
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
                  ? new Date(user.created_at).toLocaleDateString("en-US", {
                      month: "short",
                      day: "numeric",
                      year: "numeric",
                    })
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
      <span
        className={cn(
          "text-[13px] font-medium",
          valueColor ?? "text-foreground",
        )}
      >
        {value}
      </span>
    </div>
  );
});
