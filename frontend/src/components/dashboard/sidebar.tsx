import { useRouterState, Link } from "@tanstack/react-router";
import {
  LayoutDashboard,
  Server,
  Plug,
  Settings,
  BookOpen,
  BookMarked,
  Code,
  Users,
  ShieldCheck,
  UsersRound,
  KeyRound,
  Link2,
  Bot,
  Bell,
  ClipboardList,
  Lock,
  HardDrive,
  Sparkles,
  Cable,
  Radio,
  Ticket,
  Building2,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { useAuthStore } from "@/stores/auth-store";
import { hasAdminRead } from "@/types/api";

/* ── Navigation Config ── */
export const MAIN_NAV = [
  { to: "/dashboard", icon: LayoutDashboard, label: "Dashboard" },
  { to: "/keys", icon: Cable, label: "AI Services" },
  { to: "/orgs", icon: Building2, label: "Organizations" },
  { to: "/nodes", icon: HardDrive, label: "Nodes" },
  { to: "/channel-bots", icon: Radio, label: "Channel Bots" },
  { to: "/settings", icon: Settings, label: "Settings" },
  { to: "/settings/consents", icon: KeyRound, label: "Authorized Apps" },
  { to: "/settings/authorizations", icon: Link2, label: "Authorizations" },
  { to: "/guide", icon: BookOpen, label: "Guide" },
] as const;

export const APPROVALS_NAV = [
  { to: "/approvals/settings", icon: Bell, label: "Notifications" },
  { to: "/approvals/history", icon: ClipboardList, label: "Approval History" },
  { to: "/approvals/grants", icon: Lock, label: "Active Grants" },
] as const;

export const DEVELOPER_NAV = [
  { to: "/developer/apps", icon: Code, label: "Developer Apps" },
  { to: "/ai-setup", icon: Sparkles, label: "AI Setup" },
  { to: "/integration-guide", icon: BookMarked, label: "Integration Guide" },
] as const;

const ADMIN_NAV = [
  { to: "/admin/users", icon: Users, label: "Users" },
  { to: "/admin/invite-codes", icon: Ticket, label: "Invite Codes" },
  { to: "/admin/audit-log", icon: ClipboardList, label: "Audit Log" },
  { to: "/admin/service-accounts", icon: Bot, label: "Service Accounts" },
  { to: "/admin/roles", icon: ShieldCheck, label: "Roles" },
  { to: "/admin/groups", icon: UsersRound, label: "Groups" },
  { to: "/admin/nodes", icon: HardDrive, label: "Nodes" },
  { to: "/services", icon: Server, label: "Services" },
  { to: "/providers", icon: Plug, label: "Providers" },
] as const;

/** Check if a nav item is the best (most specific) match for the current path. */
export function isNavActive(
  itemTo: string,
  currentPath: string,
  allItems: readonly { readonly to: string }[],
): boolean {
  if (itemTo === "/") return currentPath === "/";
  const matches =
    currentPath === itemTo || currentPath.startsWith(itemTo + "/");
  if (!matches) return false;
  return !allItems.some(
    (other) =>
      other.to !== itemTo &&
      other.to.length > itemTo.length &&
      (currentPath === other.to || currentPath.startsWith(other.to + "/")),
  );
}

/* ── Shared nav link renderer ── */
function NavLink({
  item,
  isActive,
  onClick,
}: {
  readonly item: {
    readonly to: string;
    readonly icon: React.ComponentType<{ className?: string }>;
    readonly label: string;
  };
  readonly isActive: boolean;
  readonly onClick?: () => void;
}) {
  return (
    <Link
      to={item.to}
      onClick={onClick}
      className={cn(
        "relative flex w-full items-center gap-[14px] rounded-[10px] px-4 py-3.5 text-sm transition-colors",
        isActive
          ? "bg-primary/[0.15] font-medium text-foreground"
          : "font-normal text-muted-foreground hover:bg-accent hover:text-accent-foreground",
      )}
      style={
        isActive
          ? { boxShadow: "inset 2px 0 0 0 var(--color-primary)" }
          : undefined
      }
    >
      <item.icon
        className={cn(
          "h-[18px] w-[18px] shrink-0",
          isActive ? "text-primary" : "text-text-tertiary",
        )}
      />
      {item.label}
    </Link>
  );
}

/* ── VoidPortal Sidebar ── */
export function Sidebar({
  onNavigate,
}: { readonly onNavigate?: () => void } = {}) {
  const routerState = useRouterState();
  const user = useAuthStore((s) => s.user);
  const currentPath = routerState.location.pathname;

  /* Initials from user name or email */
  const initials = user?.display_name
    ? user.display_name
        .split(" ")
        .map((w) => w[0])
        .join("")
        .slice(0, 2)
        .toUpperCase()
    : (user?.email?.slice(0, 2).toUpperCase() ?? "U");

  return (
    <aside className="flex h-full w-[280px] flex-col overflow-y-auto border-r border-border bg-sidebar px-7 py-10">
      {/* ── Navigation ── */}
      <div className="flex flex-1 flex-col gap-6">
        {/* Logo */}
        <div className="mb-6 flex items-center">
          <img
            src="/nyxid-wordmark.svg"
            alt="NyxID"
            className="h-9 w-auto"
          />
        </div>

        {/* Main Nav */}
        <nav className="flex flex-col gap-1">
          {MAIN_NAV.map((item) => (
            <NavLink
              key={item.to}
              item={item}
              isActive={isNavActive(item.to, currentPath, MAIN_NAV)}
              onClick={onNavigate}
            />
          ))}
        </nav>

        {/* Approvals section */}
        <div className="flex flex-col gap-1">
          <p className="mb-1 px-4 text-[11px] font-semibold uppercase tracking-[1px] text-text-tertiary">
            Approvals
          </p>
          {APPROVALS_NAV.map((item) => (
            <NavLink
              key={item.to}
              item={item}
              isActive={isNavActive(item.to, currentPath, APPROVALS_NAV)}
              onClick={onNavigate}
            />
          ))}
        </div>

        {/* Developer section */}
        <div className="flex flex-col gap-1">
          <p className="mb-1 px-4 text-[11px] font-semibold uppercase tracking-[1px] text-text-tertiary">
            Developer
          </p>
          {DEVELOPER_NAV.map((item) => (
            <NavLink
              key={item.to}
              item={item}
              isActive={isNavActive(item.to, currentPath, DEVELOPER_NAV)}
              onClick={onNavigate}
            />
          ))}
        </div>

        {/* Admin section — visible to admins (read+write) and operators
            (read-only). Per-action write controls are gated separately
            inside individual admin pages. */}
        {hasAdminRead(user) && (
          <div className="flex flex-col gap-1">
            <p className="mb-1 px-4 text-[11px] font-semibold uppercase tracking-[1px] text-text-tertiary">
              Admin
            </p>
            {ADMIN_NAV.map((item) => (
              <NavLink
                key={item.to}
                item={item}
                isActive={isNavActive(item.to, currentPath, ADMIN_NAV)}
                onClick={onNavigate}
              />
            ))}
          </div>
        )}
      </div>

      {/* ── Account ── */}
      <div className="flex items-center gap-3 border-t border-border mt-6 pt-4">
        <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-full border border-primary">
          <span className="text-xs font-semibold text-void-400">
            {initials}
          </span>
        </div>
        <div className="flex min-w-0 flex-col gap-0.5">
          <span className="truncate text-[13px] font-medium text-foreground">
            {user?.display_name ?? "User"}
          </span>
          <span className="truncate text-[11px] text-text-tertiary">
            {user?.email ?? ""}
          </span>
        </div>
      </div>
    </aside>
  );
}
