import { useRouterState, useNavigate } from "@tanstack/react-router";
import { useLogout } from "@/hooks/use-auth";
import { useAuthStore } from "@/stores/auth-store";
import { sanitizeAvatarUrl } from "@/lib/utils";
import { Avatar, AvatarFallback, AvatarImage } from "@/components/ui/avatar";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuLabel,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { User, Settings, LogOut, Menu } from "lucide-react";

function getPageTitle(pathname: string): string {
  const titles: Record<string, string> = {
    "/": "Dashboard",
    "/api-keys": "API Keys",
    "/services": "Services",
    "/connections": "Connections",
    "/settings": "Settings",
  };
  const segment = "/" + (pathname.split("/")[1] ?? "");
  return titles[segment] ?? "Dashboard";
}

function getInitials(name: string | null, email: string): string {
  if (name) {
    return name
      .split(" ")
      .map((n) => n[0])
      .filter(Boolean)
      .join("")
      .toUpperCase()
      .slice(0, 2);
  }
  return email.slice(0, 2).toUpperCase();
}

/* ── VoidPortal Header ── */
export function Header({ onMenuClick }: { readonly onMenuClick?: () => void } = {}) {
  const routerState = useRouterState();
  const navigate = useNavigate();
  const logoutMutation = useLogout();
  const user = useAuthStore((s) => s.user);

  const title = getPageTitle(routerState.location.pathname);
  const safeAvatarUrl = sanitizeAvatarUrl(user?.avatar_url);

  async function handleLogout() {
    await logoutMutation.mutateAsync();
    void navigate({ to: "/login" as string });
  }

  return (
    <header className="flex h-16 items-center justify-between border-b border-border bg-background px-4 md:px-14">
      <div className="flex items-center gap-3">
        <button
          type="button"
          onClick={onMenuClick}
          className="flex h-9 w-9 items-center justify-center rounded-lg transition-colors hover:bg-accent md:hidden"
          aria-label="Open menu"
        >
          <Menu className="h-5 w-5 text-muted-foreground" />
        </button>
        <h1 className="font-display text-lg font-normal md:text-[22px]">{title}</h1>
      </div>

      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <button
            type="button"
            className="flex items-center gap-3 rounded-lg p-1.5 transition-colors hover:bg-accent"
            aria-label="User menu"
          >
            {/* Name + email right-aligned — hidden on mobile */}
            <div className="hidden flex-col items-end gap-0.5 sm:flex">
              <span className="text-[13px] font-medium text-foreground">
                {user?.name ?? user?.email ?? "User"}
              </span>
              {user?.name && (
                <span className="text-[11px] text-text-tertiary">{user.email}</span>
              )}
            </div>
            <Avatar className="h-10 w-10">
              {safeAvatarUrl && <AvatarImage src={safeAvatarUrl} alt="" />}
              <AvatarFallback className="text-xs">
                {getInitials(user?.name ?? null, user?.email ?? "")}
              </AvatarFallback>
            </Avatar>
          </button>
        </DropdownMenuTrigger>
        <DropdownMenuContent align="end" className="w-56">
          <DropdownMenuLabel>My Account</DropdownMenuLabel>
          <DropdownMenuSeparator />
          <DropdownMenuItem
            onClick={() => void navigate({ to: "/settings" as string })}
          >
            <User className="h-4 w-4" aria-hidden="true" />
            Profile
          </DropdownMenuItem>
          <DropdownMenuItem
            onClick={() => void navigate({ to: "/settings" as string })}
          >
            <Settings className="h-4 w-4" aria-hidden="true" />
            Settings
          </DropdownMenuItem>
          <DropdownMenuSeparator />
          <DropdownMenuItem
            onClick={() => void handleLogout()}
            className="text-destructive focus:text-destructive"
          >
            <LogOut className="h-4 w-4" aria-hidden="true" />
            Log out
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
    </header>
  );
}
