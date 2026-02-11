import { useRouterState, useNavigate } from "@tanstack/react-router";
import {
  LayoutDashboard,
  Key,
  Server,
  Link2,
  Plug,
  Settings,
  BookOpen,
  Shield,
  Users,
  ShieldCheck,
  UsersRound,
  KeyRound,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { useAuthStore } from "@/stores/auth-store";
import { Separator } from "@/components/ui/separator";

const NAV_ITEMS = [
  { to: "/", icon: LayoutDashboard, label: "Dashboard" },
  { to: "/api-keys", icon: Key, label: "API Keys" },
  { to: "/services", icon: Server, label: "Services" },
  { to: "/connections", icon: Link2, label: "Connections" },
  { to: "/providers", icon: Plug, label: "Providers" },
  { to: "/settings", icon: Settings, label: "Settings" },
  { to: "/settings/consents", icon: KeyRound, label: "Authorized Apps" },
  { to: "/guide", icon: BookOpen, label: "Guide" },
] as const;

const ADMIN_NAV_ITEMS = [
  { to: "/admin/users", icon: Users, label: "Users" },
  { to: "/admin/roles", icon: ShieldCheck, label: "Roles" },
  { to: "/admin/groups", icon: UsersRound, label: "Groups" },
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
  // Only highlight if no more-specific sibling also matches
  return !allItems.some(
    (other) =>
      other.to !== itemTo &&
      other.to.length > itemTo.length &&
      (currentPath === other.to || currentPath.startsWith(other.to + "/")),
  );
}

export function Sidebar() {
  const routerState = useRouterState();
  const navigate = useNavigate();
  const user = useAuthStore((s) => s.user);
  const currentPath = routerState.location.pathname;

  return (
    <aside className="flex w-64 flex-col border-r bg-card">
      <div className="flex h-16 items-center gap-2 px-6">
        <div className="flex h-8 w-8 items-center justify-center rounded-lg bg-primary">
          <Shield className="h-5 w-5 text-primary-foreground" />
        </div>
        <span className="text-lg font-bold">NyxID</span>
      </div>

      <Separator />

      <nav className="flex-1 space-y-1 p-4">
        {NAV_ITEMS.map((item) => {
          const isActive = isNavActive(item.to, currentPath, NAV_ITEMS);

          return (
            <button
              key={item.to}
              type="button"
              onClick={() => void navigate({ to: item.to as string })}
              className={cn(
                "flex w-full items-center gap-3 rounded-lg px-3 py-2 text-sm font-medium transition-colors",
                isActive
                  ? "bg-primary/10 text-primary"
                  : "text-muted-foreground hover:bg-accent hover:text-accent-foreground",
              )}
            >
              <item.icon className="h-4 w-4" />
              {item.label}
            </button>
          );
        })}
      </nav>

      {user?.is_admin && (
        <>
          <Separator />
          <div className="px-4 pt-2">
            <p className="mb-2 px-3 text-xs font-semibold uppercase tracking-wider text-muted-foreground">
              Admin
            </p>
          </div>
          <nav className="space-y-1 px-4 pb-2">
            {ADMIN_NAV_ITEMS.map((item) => {
              const isActive = isNavActive(item.to, currentPath, ADMIN_NAV_ITEMS);
              return (
                <button
                  key={item.to}
                  type="button"
                  onClick={() => void navigate({ to: item.to as string })}
                  className={cn(
                    "flex w-full items-center gap-3 rounded-lg px-3 py-2 text-sm font-medium transition-colors",
                    isActive
                      ? "bg-primary/10 text-primary"
                      : "text-muted-foreground hover:bg-accent hover:text-accent-foreground",
                  )}
                >
                  <item.icon className="h-4 w-4" />
                  {item.label}
                </button>
              );
            })}
          </nav>
        </>
      )}

      <Separator />

      <div className="p-4">
        <div className="rounded-lg bg-muted p-3">
          <p className="text-xs font-medium text-muted-foreground">
            NyxID v1.0
          </p>
          <p className="text-xs text-muted-foreground/70">
            Identity & Access Management
          </p>
        </div>
      </div>
    </aside>
  );
}
