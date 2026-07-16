import { useEffect, useState, type ReactNode } from "react";
import { Link, useNavigate } from "@tanstack/react-router";
import { LogOut, Menu, Settings, User, X } from "lucide-react";
import { NyxidLogo } from "@/components/brand/nyxid-logo";
import { ThemeToggle } from "@/components/dashboard/theme-toggle";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { useLogout } from "@/hooks/use-auth";
import { useApplyTheme } from "@/hooks/use-theme";
import { useAuthStore } from "@/stores/auth-store";

export function AssistantShell({
  title,
  sidebar,
  children,
}: {
  readonly title: string;
  readonly sidebar: ReactNode;
  readonly children: ReactNode;
}) {
  const [mobileSidebarOpen, setMobileSidebarOpen] = useState(false);
  const user = useAuthStore((state) => state.user);
  const logout = useLogout();
  const navigate = useNavigate();
  useApplyTheme();

  useEffect(() => {
    document.title = `nyxid - ${title}`;
  }, [title]);

  useEffect(() => {
    if (!mobileSidebarOpen) return;
    function closeOnEscape(event: KeyboardEvent) {
      if (event.key === "Escape") setMobileSidebarOpen(false);
    }
    window.addEventListener("keydown", closeOnEscape);
    return () => window.removeEventListener("keydown", closeOnEscape);
  }, [mobileSidebarOpen]);

  async function handleLogout() {
    await logout.mutateAsync();
    void navigate({ to: "/login" as string });
  }

  const profileMenu = (
    <DropdownMenu>
      <DropdownMenuTrigger asChild>
        <button
          type="button"
          className="flex h-8 w-8 items-center justify-center rounded-lg border border-hairline text-text-tertiary transition-colors hover:border-hairline-strong hover:text-muted-foreground focus-visible:outline-none"
          aria-label="User menu"
        >
          <User className="h-[14px] w-[14px]" />
        </button>
      </DropdownMenuTrigger>
      <DropdownMenuContent align="end" className="w-48 p-2">
        <div className="px-2 py-1.5">
          <p className="truncate text-[12px] font-medium text-foreground">
            {user?.display_name ?? "User"}
          </p>
          <p className="truncate text-[11px] text-text-tertiary">
            {user?.email ?? ""}
          </p>
        </div>
        <DropdownMenuItem
          onClick={() => void navigate({ to: "/settings" })}
          className="rounded-md text-[12px]"
        >
          <Settings />
          Settings
        </DropdownMenuItem>
        <DropdownMenuItem
          onClick={() => void handleLogout()}
          className="rounded-md text-[12px] text-destructive focus:text-destructive"
        >
          <LogOut />
          Log out
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );

  return (
    <div
      className="flex h-dvh flex-col overflow-hidden bg-background"
      style={{
        paddingTop: "var(--sat)",
        paddingLeft: "var(--sal)",
        paddingRight: "var(--sar)",
      }}
    >
      <header className="flex h-[52px] shrink-0 items-center border-b border-border/60">
        <div className="hidden h-full w-[200px] shrink-0 items-center px-4 md:flex">
          <Link to="/assistant" search={{}} aria-label="Assistant home">
            <NyxidLogo className="h-5 w-auto" />
          </Link>
        </div>
        <div className="flex min-w-0 flex-1 items-center gap-2 px-3 sm:px-4">
          <button
            type="button"
            onClick={() => setMobileSidebarOpen(true)}
            className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg border border-hairline text-text-tertiary md:hidden"
            aria-label="Open chats"
          >
            <Menu className="h-4 w-4" />
          </button>
          <div className="hidden shrink-0 text-[12px] text-text-tertiary sm:block">
            assistant
          </div>
          <span className="hidden text-[12px] text-text-tertiary sm:block">
            /
          </span>
          <div className="min-w-0 flex-1 truncate text-[12px] text-muted-foreground">
            {title}
          </div>
          <ThemeToggle className="shrink-0" />
          {profileMenu}
        </div>
      </header>

      <div className="flex min-h-0 flex-1 overflow-hidden">
        <aside className="hidden w-[200px] shrink-0 border-r border-border/60 md:block">
          {sidebar}
        </aside>
        <main className="min-w-0 flex-1 overflow-hidden">{children}</main>
      </div>

      {mobileSidebarOpen && (
        <div className="fixed inset-0 z-[80] flex md:hidden">
          <button
            type="button"
            className="absolute inset-0 bg-background/80 backdrop-blur-sm"
            onClick={() => setMobileSidebarOpen(false)}
            aria-label="Close chats"
          />
          <aside
            className="relative flex h-full w-[min(320px,88vw)] flex-col border-r border-border bg-background shadow-xl"
            style={{
              paddingTop: "var(--sat)",
              paddingBottom: "var(--sab)",
              paddingLeft: "var(--sal)",
            }}
          >
            <div className="flex h-[52px] shrink-0 items-center justify-between border-b border-border/60 px-4">
              <NyxidLogo className="h-5 w-auto" />
              <button
                type="button"
                onClick={() => setMobileSidebarOpen(false)}
                className="flex h-8 w-8 items-center justify-center rounded-lg text-text-tertiary"
                aria-label="Close chats"
              >
                <X className="h-4 w-4" />
              </button>
            </div>
            <div
              className="min-h-0 flex-1"
              onClick={(event) => {
                if ((event.target as HTMLElement).closest("a,button")) {
                  setMobileSidebarOpen(false);
                }
              }}
            >
              {sidebar}
            </div>
          </aside>
        </div>
      )}
    </div>
  );
}
