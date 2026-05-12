import { useState, useRef, useCallback } from "react";
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

export function Header({
  onMenuClick,
}: { readonly onMenuClick?: () => void } = {}) {
  const routerState = useRouterState();
  const navigate = useNavigate();
  const logoutMutation = useLogout();
  const user = useAuthStore((s) => s.user);

  const title = getPageTitle(routerState.location.pathname);
  const safeAvatarUrl = sanitizeAvatarUrl(user?.avatar_url);

  const [dropdownOpen, setDropdownOpen] = useState(false);
  const closeTimeoutRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const cancelClose = useCallback(() => {
    if (closeTimeoutRef.current) {
      clearTimeout(closeTimeoutRef.current);
      closeTimeoutRef.current = null;
    }
  }, []);

  const scheduleClose = useCallback(() => {
    cancelClose();
    closeTimeoutRef.current = setTimeout(() => {
      setDropdownOpen(false);
    }, 150);
  }, [cancelClose]);

  const handleMouseEnter = useCallback(() => {
    cancelClose();
    setDropdownOpen(true);
  }, [cancelClose]);

  async function handleLogout() {
    await logoutMutation.mutateAsync();
    void navigate({ to: "/login" as string });
  }

  return (
    <header className="flex h-14 items-center justify-between border-b border-border bg-background px-4 md:px-14">
      <div className="flex items-center gap-3">
        <button
          type="button"
          onClick={onMenuClick}
          className="flex h-9 w-9 items-center justify-center rounded-lg transition-colors duration-300 hover:bg-accent md:hidden"
          aria-label="Open menu"
        >
          <Menu className="h-5 w-5 text-muted-foreground" />
        </button>
        <h1 className="text-lg font-semibold md:text-xl">
          {title}
        </h1>
      </div>

      <DropdownMenu open={dropdownOpen} onOpenChange={setDropdownOpen}>
        <div
          onMouseEnter={handleMouseEnter}
          onMouseLeave={scheduleClose}
        >
          <DropdownMenuTrigger asChild>
            <button
              type="button"
              className="flex items-center gap-3 rounded-lg p-1.5 transition-colors duration-300 hover:bg-accent"
              aria-label="User menu"
            >
              <div className="hidden flex-col items-end gap-0.5 sm:flex">
                <span className="text-[13px] font-medium text-foreground">
                  {user?.display_name ?? user?.email ?? "User"}
                </span>
                {user?.display_name && (
                  <span className="text-[11px] text-text-tertiary">
                    {user.email}
                  </span>
                )}
              </div>
              <Avatar className="h-9 w-9">
                {safeAvatarUrl && <AvatarImage src={safeAvatarUrl} alt="" />}
                <AvatarFallback className="bg-nyx-500/10 text-xs text-nyx-300">
                  {getInitials(user?.display_name ?? null, user?.email ?? "")}
                </AvatarFallback>
              </Avatar>
            </button>
          </DropdownMenuTrigger>
        </div>
        <DropdownMenuContent
          align="end"
          className="w-56"
          onMouseEnter={cancelClose}
          onMouseLeave={scheduleClose}
        >
          <DropdownMenuLabel>My Account</DropdownMenuLabel>

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
