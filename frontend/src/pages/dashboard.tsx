import { useState, useCallback, useEffect } from "react";
import { Link } from "@tanstack/react-router";
import { useAuthStore } from "@/stores/auth-store";
import { useApiKeys } from "@/hooks/use-api-keys";
import { useKeys } from "@/hooks/use-keys";
import { useNodes } from "@/hooks/use-nodes";
import { useRightPanel } from "@/components/layout/dashboard-layout";
import { Skeleton } from "@/components/ui/skeleton";
import { Button, ButtonIcon } from "@/components/ui/button";
import {
  ArrowRight,
  ArrowUpRight,
  Building2,
  Cable,
  Check,
  KeyRound,
  Mail,
  MailCheck,
  MailX,
  Radio,
  Server,
  Shield,
  ShieldCheck,
  ShieldOff,
  Smartphone,
  Wifi,
  WifiOff,
  X,
} from "lucide-react";
import { cn } from "@/lib/utils";

const AI_SETUP_DISMISSED_KEY = "nyxid:ai-setup-dismissed";
const ONBOARDING_DISMISSED_KEY = "nyxid:onboarding-dismissed";

const MOBILE_APP_LINK = "https://nyxid.onelink.me/REzJ/dql9w8fx";

export function DashboardPage() {
  const user = useAuthStore((s) => s.user);
  const { data: apiKeys, isLoading: keysLoading } = useApiKeys();
  const { data: services, isLoading: servicesLoading } = useKeys();
  const { data: nodes, isLoading: nodesLoading } = useNodes();
  const { setRightPanel } = useRightPanel();

  const activeKeys = apiKeys?.filter((k) => k.is_active).length ?? 0;
  const serviceCount = services?.length ?? 0;
  const onlineNodes = nodes?.filter((n) => n.status === "Online").length ?? 0;
  const totalNodes = nodes?.length ?? 0;

  const [aiDismissed, setAiDismissed] = useState(
    () => localStorage.getItem(AI_SETUP_DISMISSED_KEY) === "true",
  );
  const dismissAi = useCallback(() => {
    localStorage.setItem(AI_SETUP_DISMISSED_KEY, "true");
    setAiDismissed(true);
  }, []);

  const [onboardingDismissed, setOnboardingDismissed] = useState(
    () => localStorage.getItem(ONBOARDING_DISMISSED_KEY) === "true",
  );
  const dismissOnboarding = useCallback(() => {
    localStorage.setItem(ONBOARDING_DISMISSED_KEY, "true");
    setOnboardingDismissed(true);
  }, []);

  useEffect(() => {
    setRightPanel(
      <>
        {!aiDismissed && <AiSetupCard onDismiss={dismissAi} />}
        <RightPanelContent />
      </>,
    );
    return () => setRightPanel(null);
  }, [setRightPanel, aiDismissed, dismissAi]);

  if (servicesLoading) {
    return (
      <div className="flex flex-col gap-8 px-4 pt-8 sm:px-6 md:px-8 lg:px-10">
        <Skeleton className="h-8 w-48" />
        <div className="grid grid-cols-1 sm:grid-cols-2 gap-3">
          <Skeleton className="h-16 w-full rounded-xl" />
          <Skeleton className="h-16 w-full rounded-xl" />
          <Skeleton className="h-16 w-full rounded-xl" />
          <Skeleton className="h-16 w-full rounded-xl" />
        </div>
      </div>
    );
  }

  const emailVerified = !!user?.email_verified;
  const mfaEnabled = !!user?.mfa_enabled;

  return (
    <div className="flex flex-col gap-8">
      {/* Onboarding checklist — guides remaining steps after first service */}
      {!onboardingDismissed && activeKeys === 0 && (
        <OnboardingChecklist
          serviceConnected={serviceCount > 0}
          activeKeys={activeKeys}
          loading={keysLoading}
          onDismiss={dismissOnboarding}
        />
      )}

      {/* Greeting */}
      <div>
        <h1
          className="text-[22px] sm:text-[28px] font-bold leading-[1.1]"
          style={{ letterSpacing: "-0.03em" }}
        >
          Welcome back, {user?.display_name ?? "there"}
        </h1>
        <p className="text-[12px] text-muted-foreground mt-1">
          {user?.email ?? ""}
        </p>
      </div>

      {/* Status grid + account card */}
      <div className="flex flex-col md:flex-row gap-4">
        {/* Left: status grid */}
        <div className="flex-1 grid grid-cols-1 sm:grid-cols-2 gap-3">
          <StatusCell
            icon={
              emailVerified ? (
                <MailCheck className="h-4 w-4" />
              ) : (
                <MailX className="h-4 w-4" />
              )
            }
            iconColor={emailVerified ? "text-success" : "text-destructive"}
            label="Email"
            value={emailVerified ? "Verified" : "Unverified"}
            loading={false}
            href="/settings"
          />
          <StatusCell
            icon={
              mfaEnabled ? (
                <ShieldCheck className="h-4 w-4" />
              ) : (
                <ShieldOff className="h-4 w-4" />
              )
            }
            iconColor={mfaEnabled ? "text-success" : "text-destructive"}
            label="MFA"
            value={mfaEnabled ? "Enabled" : "Disabled"}
            loading={false}
            href="/settings?tab=security"
          />
          <StatusCell
            icon={<Server className="h-4 w-4" />}
            iconColor="text-muted-foreground"
            label="Services"
            value={serviceCount > 0 ? `${String(serviceCount)} connected` : "No services"}
            loading={servicesLoading}
            href="/keys"
          />
          <StatusCell
            icon={<KeyRound className="h-4 w-4" />}
            iconColor="text-muted-foreground"
            label="API Keys"
            value={activeKeys > 0 ? `${String(activeKeys)} active` : "No keys"}
            loading={keysLoading}
            href="/keys?tab=nyxid"
          />
          <StatusCell
            icon={
              onlineNodes > 0 ? (
                <Wifi className="h-4 w-4" />
              ) : (
                <WifiOff className="h-4 w-4" />
              )
            }
            iconColor={onlineNodes > 0 ? "text-success" : "text-muted-foreground"}
            label="Nodes"
            value={
              totalNodes > 0
                ? `${String(onlineNodes)}/${String(totalNodes)} online`
                : "No nodes"
            }
            loading={nodesLoading}
            href="/nodes"
          />
          <StatusCell
            icon={<Shield className="h-4 w-4" />}
            iconColor="text-muted-foreground"
            label="Approvals"
            value="Configure"
            loading={false}
            href="/approvals/settings"
          />
        </div>

        {/* Right: account posture card */}
        <AccountPostureCard
          emailVerified={emailVerified}
          mfaEnabled={mfaEnabled}
          serviceCount={serviceCount}
          activeKeys={activeKeys}
          loading={servicesLoading || keysLoading}
        />
      </div>

      {/* Shortcuts */}
      <div>
        <h2 className="text-[15px] font-semibold text-foreground mb-3">
          Shortcuts
        </h2>
        <div className="grid grid-cols-2 sm:grid-cols-3 lg:grid-cols-5 gap-3">
          <QuickActionCard
            icon={<Server className="h-4 w-4" />}
            title="Services"
            desc="Connect an API"
            href="/keys?action=add-service"
          />
          <QuickActionCard
            icon={<KeyRound className="h-4 w-4" />}
            title="API Keys"
            desc="Create a scoped key"
            href="/keys?tab=nyxid&action=create-key"
          />
          <QuickActionCard
            icon={<Wifi className="h-4 w-4" />}
            title="Nodes"
            desc="Register a node"
            href="/nodes"
          />
          <QuickActionCard
            icon={<Building2 className="h-4 w-4" />}
            title="Organizations"
            desc="Manage your orgs"
            href="/orgs"
          />
          <QuickActionCard
            icon={<Radio className="h-4 w-4" />}
            title="Channel Bots"
            desc="Set up a bot"
            href="/channel-bots"
          />
        </div>
      </div>

      {/* Right panel content — inline on mobile/tablet, hidden on lg+ (shown in sidebar) */}
      <div className="flex flex-col gap-4 lg:hidden">
        {!aiDismissed && <AiSetupCard onDismiss={dismissAi} />}
        <ApprovalsCard />
        <div className="rounded-xl border border-border/50 bg-card p-4 flex flex-col gap-2.5">
          <p className="text-[10px] font-semibold uppercase tracking-[1.5px] text-text-tertiary">
            Quick Links
          </p>
          <div className="flex flex-col gap-1.5">
            <QuickLink to="/docs" label="Documentation" />
            <QuickLink to="/ai-setup" label="AI Setup Guide" />
            <QuickLink to="/integration-guide" label="Integration Guide" />
          </div>
        </div>
      </div>
    </div>
  );
}

/* ─────────────── Onboarding checklist ─────────────── */

function OnboardingChecklist({
  serviceConnected,
  activeKeys,
  loading,
  onDismiss,
}: {
  readonly serviceConnected: boolean;
  readonly activeKeys: number;
  readonly loading: boolean;
  readonly onDismiss: () => void;
}) {
  const steps = [
    {
      done: serviceConnected,
      title: "Connect a Service",
      description: "Add an API service to proxy through NyxID.",
      icon: <Cable className="h-4 w-4" />,
      href: "/keys?action=add-service",
      cta: "Add service",
    },
    {
      done: activeKeys > 0,
      title: "Create an Agent Key",
      description: "Generate a scoped key for your AI agent.",
      icon: <KeyRound className="h-4 w-4" />,
      href: "/keys?tab=nyxid&action=create-key",
      cta: "Create Key",
    },
    {
      done: false,
      title: "Make first proxy call",
      description: "Use your agent key to route a request through NyxID.",
      icon: <Wifi className="h-4 w-4" />,
      href: "/keys",
      cta: "Make a call",
    },
  ];

  return (
    <div>
      <div className="flex items-center justify-between mb-4">
        <div>
          <h2 className="text-[15px] font-semibold text-foreground">
            Getting started
          </h2>
          <p className="text-[11px] text-muted-foreground mt-0.5">
            {steps.filter((s) => s.done).length} of {steps.length} complete
          </p>
        </div>
        <button
          type="button"
          onClick={onDismiss}
          className="flex h-6 w-6 items-center justify-center rounded-md text-text-tertiary/40 hover:text-foreground transition-colors"
          aria-label="Dismiss"
        >
          <X className="h-3.5 w-3.5" />
        </button>
      </div>

      {loading ? (
        <div className="flex flex-col gap-4 md:grid md:grid-cols-3">
          <Skeleton className="h-24 w-full rounded-xl" />
          <Skeleton className="h-24 w-full rounded-xl" />
          <Skeleton className="h-24 w-full rounded-xl" />
        </div>
      ) : (
        <>
          {/* ── Mobile: left-aligned timeline ── */}
          <div className="flex flex-col md:hidden">
            {steps.map((step, i) => (
              <div key={i}>
                {/* Connector line between icons */}
                {i > 0 && (
                  <div className="flex">
                    <div className="flex w-[36px] shrink-0 justify-center py-0">
                      <div className={cn("w-[2px] h-10 -my-3.5", steps[i - 1]?.done ? "bg-nyx-secondary-400/60" : "bg-border/40")} />
                    </div>
                  </div>
                )}

                {/* Row: icon + card */}
                <div className="flex items-center gap-4">
                  <div
                    className={cn(
                      "flex h-[36px] w-[36px] shrink-0 items-center justify-center rounded-lg border transition-all duration-200",
                      step.done || i === 0 || steps[i - 1]?.done
                        ? "border-nyx-secondary-400 bg-nyx-500/15 text-nyx-secondary-400"
                        : "border-border/60 bg-card text-text-tertiary",
                    )}
                  >
                    {step.icon}
                  </div>

                  <Link to={step.href} className="group flex-1">
                    <div
                      className={cn(
                        "rounded-xl border px-4 py-3.5 transition-all duration-200",
                        step.done
                          ? "border-nyx-500/30 bg-nyx-500/[0.06]"
                          : "border-border/50 bg-card group-active:bg-white/[0.03]",
                      )}
                    >
                      <p className={cn("text-[13px] font-semibold", step.done ? "text-foreground/50" : "text-foreground")}>
                        {step.title}
                      </p>
                      <p className={cn("text-[11px] mt-0.5 leading-relaxed", step.done ? "text-muted-foreground/50" : "text-muted-foreground")}>
                        {step.description}
                      </p>
                      {step.done ? (
                        <span className="mt-2 inline-flex items-center gap-1.5 text-[11px] font-medium text-success/70">
                          <Check className="h-3 w-3" />
                          Completed
                        </span>
                      ) : (
                        <span className="mt-2 inline-flex items-center gap-1.5 text-[11px] font-semibold text-nyx-secondary-400">
                          {step.cta}
                          <ArrowRight className="h-3 w-3" />
                        </span>
                      )}
                    </div>
                  </Link>
                </div>
              </div>
            ))}
          </div>

          {/* ── Desktop: horizontal cards ── */}
          <div className="relative hidden md:block">
            <div className="absolute top-[19px] left-[calc(16.67%+27px)] right-[calc(16.67%+27px)] flex items-center gap-[54px] z-0">
              <div className={cn("h-[2px] flex-1", steps[0]?.done ? "bg-nyx-secondary-400/60" : "bg-border/40")} />
              <div className={cn("h-[2px] flex-1", steps[1]?.done ? "bg-nyx-secondary-400/60" : "bg-border/40")} />
            </div>

            <div className="relative z-10 grid grid-cols-3 gap-4">
              {steps.map((step, i) => (
                <Link key={i} to={step.href} className="group flex flex-col items-center">
                  <div
                    className={cn(
                      "flex h-[38px] w-[38px] items-center justify-center rounded-lg border transition-all duration-200",
                      step.done || i === 0 || steps[i - 1]?.done
                        ? "border-nyx-secondary-400 bg-nyx-500/15 text-nyx-secondary-400 group-hover:bg-nyx-500/25 group-hover:shadow-md group-hover:shadow-nyx-500/10"
                        : "border-border/60 bg-card text-text-tertiary",
                    )}
                  >
                    {step.icon}
                  </div>
                  <div
                    className={cn(
                      "relative mt-3 w-full flex flex-col items-center rounded-xl border px-4 py-4 min-h-[120px] text-center transition-all duration-200",
                      step.done
                        ? "border-nyx-500/30 bg-nyx-500/[0.06]"
                        : "border-border/50 bg-card group-hover:border-white/[0.15] group-hover:bg-white/[0.03]",
                    )}
                  >
                    <p className={cn("text-[13px] font-semibold", step.done ? "text-foreground/50" : "text-foreground")}>
                      {step.title}
                    </p>
                    <p className={cn("text-[11px] mt-1 leading-relaxed", step.done ? "text-muted-foreground/50" : "text-muted-foreground")}>{step.description}</p>
                    {step.done ? (
                      <span className="mt-3 inline-flex items-center gap-1.5 text-[11px] font-medium text-success/70">
                        <Check className="h-3 w-3" />
                        Completed
                      </span>
                    ) : (
                      <span className="mt-3 inline-flex items-center gap-1.5 text-[11px] font-semibold text-nyx-secondary-400 group-hover:gap-2 transition-all">
                        {step.cta}
                        <ArrowRight className="h-3 w-3" />
                      </span>
                    )}
                  </div>
                </Link>
              ))}
            </div>
          </div>
        </>
      )}
    </div>
  );
}

/* ─────────────── Status grid cell ─────────────── */

function StatusCell({
  icon,
  iconColor,
  label,
  value,
  loading,
  href,
}: {
  readonly icon: React.ReactNode;
  readonly iconColor: string;
  readonly label: string;
  readonly value: string;
  readonly loading: boolean;
  readonly href: string;
}) {
  return (
    <Link
      to={href}
      className="group flex h-full items-center gap-3 rounded-xl border border-border/50 bg-card px-4 py-3 transition-colors duration-200 hover:bg-white/[0.03]"
    >
      <div
        className={cn(
          "flex h-8 w-8 shrink-0 items-center justify-center rounded-lg border border-border/50 bg-white/[0.03] transition-colors duration-200 group-hover:border-white/[0.15]",
          iconColor,
        )}
      >
        {icon}
      </div>
      <div className="min-w-0">
        <p className="truncate text-[10px] font-semibold uppercase tracking-wider text-text-tertiary">
          {label}
        </p>
        {loading ? (
          <Skeleton className="mt-1 h-4 w-20" />
        ) : (
          <p className="text-[13px] font-medium text-foreground truncate">
            {value}
          </p>
        )}
      </div>
    </Link>
  );
}

/* ─────────────── Account posture card ─────────────── */

function AccountPostureCard({
  emailVerified,
  mfaEnabled,
  serviceCount,
  activeKeys,
  loading,
}: {
  readonly emailVerified: boolean;
  readonly mfaEnabled: boolean;
  readonly serviceCount: number;
  readonly activeKeys: number;
  readonly loading: boolean;
}) {
  const items = [
    {
      done: emailVerified,
      label: "Email verified",
      icon: <Mail className="h-3.5 w-3.5" />,
      href: "/settings",
      cta: "Verify",
    },
    {
      done: mfaEnabled,
      label: "MFA enabled",
      icon: <Shield className="h-3.5 w-3.5" />,
      href: "/settings?tab=security",
      cta: "Enable",
    },
    {
      done: serviceCount > 0,
      label: "Service connected",
      icon: <Server className="h-3.5 w-3.5" />,
      href: serviceCount > 0 ? "/keys" : "/keys?action=add-service",
      cta: "Connect",
    },
    {
      done: activeKeys > 0,
      label: "API key created",
      icon: <KeyRound className="h-3.5 w-3.5" />,
      href:
        activeKeys > 0
          ? "/keys?tab=nyxid"
          : "/keys?tab=nyxid&action=create-key",
      cta: "Create",
    },
  ];
  const doneCount = items.filter((i) => i.done).length;
  const score = Math.round((doneCount / items.length) * 100);

  const statusLabel =
    score === 100
      ? "Fully Secured"
      : score >= 50
        ? "Needs Attention"
        : "At Risk";
  const statusColor =
    score === 100
      ? "text-success"
      : score >= 50
        ? "text-warning"
        : "text-destructive";

  const nextStep = items.find((i) => !i.done);

  return (
    <div className="hidden md:flex w-[280px] shrink-0 flex-col rounded-xl border border-border/50 bg-card overflow-hidden">
      {/* Header */}
      <div className="flex items-center gap-2.5 border-b border-border/50 px-4 py-3">
        <div className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg border border-nyx-500/20 bg-nyx-500/10">
          <ShieldCheck className="h-4 w-4 text-nyx-secondary-400" />
        </div>
        <div>
          <p className="text-[13px] font-semibold text-foreground">
            Security Posture
          </p>
          <p className={cn("text-[11px] font-medium", statusColor)}>
            {statusLabel}
          </p>
        </div>
      </div>

      {/* Checklist */}
      <div className="flex-1 px-2 py-2 flex flex-col">
        {loading ? (
          <div className="flex flex-col gap-3.5 px-2 py-2">
            <Skeleton className="h-4 w-full" />
            <Skeleton className="h-4 w-3/4" />
            <Skeleton className="h-4 w-full" />
            <Skeleton className="h-4 w-2/3" />
          </div>
        ) : (
          items.map((item) => (
            <Link
              key={item.label}
              to={item.href}
              className="group flex items-center gap-2.5 rounded-md px-2 py-2 transition-colors duration-200 hover:bg-white/[0.03]"
            >
              <span
                className={cn(
                  "shrink-0",
                  item.done ? "text-success" : "text-muted-foreground/40",
                )}
              >
                {item.icon}
              </span>
              <span
                className={cn(
                  "flex-1 truncate text-[12px]",
                  item.done ? "text-foreground" : "text-muted-foreground",
                )}
              >
                {item.label}
              </span>
              {item.done ? (
                <Check className="h-3 w-3 shrink-0 text-success/60 transition-transform duration-200 group-hover:scale-110" />
              ) : (
                <span className="inline-flex shrink-0 items-center gap-0.5 text-[10px] font-semibold uppercase tracking-wide text-nyx-secondary-400 transition-all duration-200 group-hover:gap-1">
                  {item.cta}
                  <ArrowRight className="h-2.5 w-2.5" />
                </span>
              )}
            </Link>
          ))
        )}
      </div>

      {/* Footer with progress bar + next-step CTA */}
      <div className="border-t border-border/50 px-4 py-2.5">
        <div className="flex items-center justify-between mb-2">
          <span className="text-[11px] text-text-tertiary">
            {doneCount} of {items.length}
          </span>
          <span className="text-[11px] font-medium text-foreground">
            {score}%
          </span>
        </div>
        <div className="h-1.5 w-full rounded-full bg-white/[0.06] overflow-hidden">
          <div
            className="h-full rounded-full nyx-gradient-vivid transition-[width] duration-700 ease-out"
            style={{ width: `${String(score)}%` }}
          />
        </div>
        {nextStep && !loading && (
          <Link
            to={nextStep.href}
            className="mt-3 group flex items-center justify-between rounded-md -mx-1 px-1 py-1 text-[11px] transition-colors duration-200 hover:bg-white/[0.03]"
          >
            <span className="text-muted-foreground">
              Next: <span className="text-foreground">{nextStep.label}</span>
            </span>
            <ArrowRight className="h-3 w-3 text-nyx-secondary-400 transition-transform duration-200 group-hover:translate-x-0.5" />
          </Link>
        )}
      </div>
    </div>
  );
}

/* ─────────────── Quick action card ─────────────── */

function QuickActionCard({
  icon,
  title,
  desc,
  href,
}: {
  readonly icon: React.ReactNode;
  readonly title: string;
  readonly desc: string;
  readonly href: string;
}) {
  return (
    <Link
      to={href}
      className="group flex flex-col items-center gap-2 rounded-xl border border-border/50 bg-card px-3 py-4 text-center transition-all duration-200 hover:border-white/[0.15] hover:bg-white/[0.03]"
    >
      <div className="flex h-8 w-8 items-center justify-center rounded-lg border border-border/50 bg-white/[0.03] text-muted-foreground transition-colors duration-200 group-hover:text-foreground group-hover:border-white/[0.15]">
        {icon}
      </div>
      <div>
        <p className="text-[12px] font-semibold text-foreground">{title}</p>
        <p className="text-[10px] text-muted-foreground mt-0.5">{desc}</p>
      </div>
    </Link>
  );
}

/* ─────────────── Right panel ─────────────── */

function RightPanelContent() {
  return (
    <>
      <ApprovalsCard />
      <div className="rounded-xl border border-border/50 bg-card p-4 flex flex-col gap-2.5">
        <p className="text-[10px] font-semibold uppercase tracking-[1.5px] text-text-tertiary">
          Quick Links
        </p>
        <div className="flex flex-col gap-1.5">
          <QuickLink to="/docs" label="Documentation" />
          <QuickLink to="/ai-setup" label="AI Setup Guide" />
          <QuickLink to="/integration-guide" label="Integration Guide" />
        </div>
      </div>
    </>
  );
}

function AiSetupCard({ onDismiss }: { readonly onDismiss: () => void }) {
  return (
    <div className="group relative overflow-hidden rounded-xl border border-nyx-500/20 transition-[border-color] duration-300 hover:border-nyx-500/40 dark:border-nyx-500/30 dark:hover:border-nyx-500/50">
      <div className="absolute inset-0 bg-gradient-to-b from-nyx-500/[0.04] via-nyx-500/[0.02] to-transparent dark:from-nyx-500/15 dark:via-nyx-500/5 dark:to-transparent" />
      <div className="absolute top-0 left-1/2 -translate-x-1/2 w-[200%] h-[120px] bg-[radial-gradient(ellipse_at_50%_0%,rgba(90,42,241,0.07)_0%,transparent_70%)] dark:bg-[radial-gradient(ellipse_at_50%_0%,rgba(90,42,241,0.25)_0%,transparent_70%)]" />
      <div className="relative flex flex-col gap-3 p-4">
        <span className="inline-flex w-fit items-center rounded-md bg-nyx-100 px-2 py-0.5 text-[10px] font-semibold text-nyx-700 dark:bg-nyx-500/20 dark:text-nyx-secondary-400">
          NEW
        </span>
        <h3 className="text-[15px] font-bold text-foreground leading-snug">
          Give your AI agents superpowers
        </h3>
        <p className="text-[12px] text-muted-foreground leading-relaxed">
          Install NyxID skills in Claude Code, Cursor, or Codex to unlock
          secure credential brokering.
        </p>
        <Button
          variant="primary"
          asChild
          className="w-full transition-transform duration-300 group-hover:-translate-y-0.5"
        >
          <Link to="/ai-setup">
            <ButtonIcon variant="primary">
              <ArrowRight className="h-3 w-3" />
            </ButtonIcon>
            Set up
          </Link>
        </Button>
        <button
          type="button"
          onClick={onDismiss}
          className="absolute right-3 top-3 flex h-6 w-6 items-center justify-center rounded-md text-text-tertiary/60 hover:text-foreground transition-colors duration-300"
          aria-label="Dismiss"
        >
          <X className="h-3 w-3" />
        </button>
      </div>
    </div>
  );
}

function QuickLink({
  to,
  label,
}: {
  readonly to: string;
  readonly label: string;
}) {
  return (
    <Link
      to={to}
      className="flex items-center justify-between rounded-lg px-2 py-1.5 -mx-2 text-[12px] text-muted-foreground transition-colors duration-300 hover:bg-white/[0.03] hover:text-foreground"
    >
      {label}
      <ArrowUpRight className="h-3 w-3 text-text-tertiary" />
    </Link>
  );
}

function ApprovalsCard() {
  return (
    <div className="rounded-xl border border-border/50 bg-card p-4 flex flex-col gap-3">
      <p className="text-[10px] font-semibold uppercase tracking-[1.5px] text-text-tertiary">
        Approvals
      </p>
      <p className="text-[12px] text-muted-foreground leading-relaxed">
        Approve AI agent access via Telegram or the NyxID mobile app.
      </p>
      <div className="flex flex-col gap-2">
        <Button asChild>
          <Link to="/approvals/settings">
            <ButtonIcon>
              <svg
                className="h-3 w-3"
                viewBox="0 0 24 24"
                fill="currentColor"
              >
                <path d="M11.944 0A12 12 0 0 0 0 12a12 12 0 0 0 12 12 12 12 0 0 0 12-12A12 12 0 0 0 12 0a12 12 0 0 0-.056 0zm4.962 7.224c.1-.002.321.023.465.14a.506.506 0 0 1 .171.325c.016.093.036.306.02.472-.18 1.898-.962 6.502-1.36 8.627-.168.9-.499 1.201-.82 1.23-.696.065-1.225-.46-1.9-.902-1.056-.693-1.653-1.124-2.678-1.8-1.185-.78-.417-1.21.258-1.91.177-.184 3.247-2.977 3.307-3.23.007-.032.014-.15-.056-.212s-.174-.041-.249-.024c-.106.024-1.793 1.14-5.061 3.345-.48.33-.913.49-1.302.48-.428-.008-1.252-.241-1.865-.44-.752-.245-1.349-.374-1.297-.789.027-.216.325-.437.893-.663 3.498-1.524 5.83-2.529 6.998-3.014 3.332-1.386 4.025-1.627 4.476-1.635z" />
              </svg>
            </ButtonIcon>
            Connect Telegram
          </Link>
        </Button>
        <Button variant="primary" asChild>
          <a href={MOBILE_APP_LINK} target="_blank" rel="noopener noreferrer">
            <ButtonIcon className="border-white/20 bg-white/10">
              <Smartphone className="h-3 w-3" />
            </ButtonIcon>
            Get App
          </a>
        </Button>
      </div>
    </div>
  );
}
