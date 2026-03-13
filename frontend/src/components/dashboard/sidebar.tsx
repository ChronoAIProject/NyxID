import { useRouterState, Link } from "@tanstack/react-router";
import {
  LayoutDashboard,
  Key,
  Server,
  Link2,
  Plug,
  Settings,
  BookOpen,
  BookMarked,
  Code,
  Users,
  ShieldCheck,
  UsersRound,
  KeyRound,
  Bot,
  Bell,
  ClipboardList,
  Lock,
  HardDrive,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { useAuthStore } from "@/stores/auth-store";
import { PortalMarkLogo } from "@/components/shared/portal-mark-logo";

/* ── Navigation Config ── */
const NAV_ITEMS = [
  { to: "/", icon: LayoutDashboard, label: "Dashboard" },
  { to: "/api-keys", icon: Key, label: "API Keys" },
  { to: "/services", icon: Server, label: "Services" },
  { to: "/connections", icon: Link2, label: "Connections" },
  { to: "/providers", icon: Plug, label: "Providers" },
  { to: "/nodes", icon: HardDrive, label: "Nodes" },
  { to: "/settings", icon: Settings, label: "Settings" },
  { to: "/settings/consents", icon: KeyRound, label: "Authorized Apps" },
  { to: "/guide", icon: BookOpen, label: "Guide" },
] as const;

const APPROVAL_NAV_ITEMS = [
  { to: "/approvals/settings", icon: Bell, label: "Notifications" },
  { to: "/approvals/history", icon: ClipboardList, label: "Approval History" },
  { to: "/approvals/grants", icon: Lock, label: "Active Grants" },
] as const;

const DEVELOPER_NAV_ITEMS = [
  { to: "/developer/apps", icon: Code, label: "Developer Apps" },
  { to: "/integration-guide", icon: BookMarked, label: "Integration Guide" },
] as const;

const ADMIN_NAV_ITEMS = [
  { to: "/admin/users", icon: Users, label: "Users" },
  { to: "/admin/service-accounts", icon: Bot, label: "Service Accounts" },
  { to: "/admin/roles", icon: ShieldCheck, label: "Roles" },
  { to: "/admin/groups", icon: UsersRound, label: "Groups" },
  { to: "/admin/nodes", icon: HardDrive, label: "Nodes" },
] as const;

/** Check if a nav item is the best (most specific) match for the current path. */
function isNavActive(
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
  readonly item: { readonly to: string; readonly icon: React.ComponentType<{ className?: string }>; readonly label: string };
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
      style={isActive ? { boxShadow: "inset 2px 0 0 0 var(--color-primary)" } : undefined}
    >
      <item.icon className={cn("h-[18px] w-[18px] shrink-0", isActive ? "text-primary" : "text-text-tertiary")} />
      {item.label}
    </Link>
  );
}

/* ── VoidPortal Sidebar ── */
export function Sidebar({ onNavigate }: { readonly onNavigate?: () => void } = {}) {
  const routerState = useRouterState();
  const user = useAuthStore((s) => s.user);
  const currentPath = routerState.location.pathname;

  /* Initials from user name or email */
  const initials = user?.name
    ? user.name.split(" ").map((w) => w[0]).join("").slice(0, 2).toUpperCase()
    : user?.email?.slice(0, 2).toUpperCase() ?? "U";

  return (
    <aside className="flex h-full w-[280px] flex-col justify-between overflow-y-auto border-r border-border bg-sidebar px-7 py-10">
      {/* ── Top: Logo + Navigation ── */}
      <div className="flex flex-col gap-12">
        {/* Logo */}
        <div className="flex items-center gap-3">
          <PortalMarkLogo size={36} className="shrink-0" />
          <span className="logo-wordmark text-[22px]">NyxID</span>
        </div>

        {/* Main Nav */}
        <nav className="flex flex-col gap-1">
          {NAV_ITEMS.map((item) => (
            <NavLink
              key={item.to}
              item={item}
              isActive={isNavActive(item.to, currentPath, NAV_ITEMS)}
              onClick={onNavigate}
            />
          ))}
        </nav>
      </div>

      {/* ── Bottom: Developer + Admin + Account ── */}
      <div className="flex flex-col gap-6">
        {/* Approvals section */}
        <div className="flex flex-col gap-1">
          <p className="mb-1 px-4 text-[11px] font-semibold uppercase tracking-[1px] text-text-tertiary">
            Approvals
          </p>
          {APPROVAL_NAV_ITEMS.map((item) => (
            <NavLink
              key={item.to}
              item={item}
              isActive={isNavActive(item.to, currentPath, APPROVAL_NAV_ITEMS)}
              onClick={onNavigate}
            />
          ))}
        </div>

        {/* Developer section */}
        <div className="flex flex-col gap-1">
          <p className="mb-1 px-4 text-[11px] font-semibold uppercase tracking-[1px] text-text-tertiary">
            Developer
          </p>
          {DEVELOPER_NAV_ITEMS.map((item) => (
            <NavLink
              key={item.to}
              item={item}
              isActive={isNavActive(item.to, currentPath, DEVELOPER_NAV_ITEMS)}
              onClick={onNavigate}
            />
          ))}
        </div>

        {/* Admin section */}
        {user?.is_admin && (
          <div className="flex flex-col gap-1">
            <p className="mb-1 px-4 text-[11px] font-semibold uppercase tracking-[1px] text-text-tertiary">
              Admin
            </p>
            {ADMIN_NAV_ITEMS.map((item) => (
              <NavLink
                key={item.to}
                item={item}
                isActive={isNavActive(item.to, currentPath, ADMIN_NAV_ITEMS)}
                onClick={onNavigate}
              />
            ))}
          </div>
        )}

        {/* Account row */}
        <div className="flex items-center gap-3 border-t border-border pt-4">
          <div className="flex h-10 w-10 shrink-0 items-center justify-center rounded-full border border-primary">
            <span className="text-xs font-semibold text-void-400">{initials}</span>
          </div>
          <div className="flex min-w-0 flex-col gap-0.5">
            <span className="truncate text-[13px] font-medium text-foreground">
              {user?.name ?? "User"}
            </span>
            <span className="truncate text-[11px] text-text-tertiary">
              {user?.email ?? ""}
            </span>
          </div>
        </div>
      </div>
    </aside>
  );
}
