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
import { User, Settings, LogOut } from "lucide-react";

function getPageTitle(pathname: string): string {
  const titles: Record<string, string> = {
    "/": "Dashboard",
    "/api-keys": "API Keys",
    "/services": "Services",
    "/connections": "Connections",
    "/settings": "Settings",
  };
  return titles[pathname] ?? "Dashboard";
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

export function Header() {
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
    <header className="flex h-16 items-center justify-between border-b px-6">
      <h1 className="text-xl font-semibold">{title}</h1>

      <DropdownMenu>
        <DropdownMenuTrigger asChild>
          <button
            type="button"
            className="flex items-center gap-3 rounded-lg p-1.5 transition-colors hover:bg-accent"
            aria-label="User menu"
          >
            <div className="text-right">
              <p className="text-sm font-medium">
                {user?.name ?? user?.email ?? "User"}
              </p>
              {user?.name && (
                <p className="text-xs text-muted-foreground">{user.email}</p>
              )}
            </div>
            <Avatar className="h-8 w-8">
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
            <User className="mr-2 h-4 w-4" aria-hidden="true" />
            Profile
          </DropdownMenuItem>
          <DropdownMenuItem
            onClick={() => void navigate({ to: "/settings" as string })}
          >
            <Settings className="mr-2 h-4 w-4" aria-hidden="true" />
            Settings
          </DropdownMenuItem>
          <DropdownMenuSeparator />
          <DropdownMenuItem
            onClick={() => void handleLogout()}
            className="text-destructive focus:text-destructive"
          >
            <LogOut className="mr-2 h-4 w-4" aria-hidden="true" />
            Log out
          </DropdownMenuItem>
        </DropdownMenuContent>
      </DropdownMenu>
    </header>
  );
}
