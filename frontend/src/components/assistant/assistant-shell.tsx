import { ThreadTitle } from "@/components/assistant/thread-title";
import { useEffect, useState, type ReactNode } from "react";
import { OverlayLayer } from "@/components/ui/overlay-layer";
import { ASSISTANT_OVERLAY_BASE } from "@/lib/overlay-layer";
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
import { useThemeStore } from "@/stores/theme-store";
import { SidebarResizeHandle } from "@/components/layout/sidebar-resize-handle";

export function AssistantShell({
  title,
  sidebar,
  headerActions,
  onRenameTitle,
  titleKey,
  children,
}: {
  readonly title: string;
  readonly sidebar: ReactNode;
  readonly headerActions?: ReactNode;
  readonly onRenameTitle?: (title: string) => Promise<void>;
  readonly titleKey?: string;
  readonly children: ReactNode;
}) {
  const [mobileSidebarOpen, setMobileSidebarOpen] = useState(false);
  const savedSidebarWidth = useThemeStore((s) => s.sidebarWidths.assistant);
  const [previewWidth, setPreviewWidth] = useState<number | null>(null);
  const sidebarWidth = previewWidth ?? savedSidebarWidth;
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
      // Radix layers opened inside the drawer (the row menu, its delete
      // confirmation) handle Escape on `document` in the capture phase and
      // preventDefault, well before this window-level listener. Escape must
      // dismiss the innermost layer only -- taking the drawer down with it
      // would unmount that layer's anchor and skip a whole step of the way
      // back in one keypress.
      if (event.defaultPrevented) return;
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
          <p className="truncate text-12 font-medium text-foreground">
            {user?.display_name ?? "User"}
          </p>
          <p className="truncate text-11 text-text-tertiary">
            {user?.email ?? ""}
          </p>
        </div>
        <DropdownMenuItem
          onClick={() => void navigate({ to: "/settings" })}
          className="rounded-md text-12"
        >
          <Settings />
          Settings
        </DropdownMenuItem>
        <DropdownMenuItem
          onClick={() => void handleLogout()}
          className="rounded-md text-12 text-destructive focus:text-destructive"
        >
          <LogOut />
          Log out
        </DropdownMenuItem>
      </DropdownMenuContent>
    </DropdownMenu>
  );

  return (
    <OverlayLayer layer={ASSISTANT_OVERLAY_BASE}>
    <div
      className="flex h-dvh flex-col overflow-hidden bg-background"
      style={{
        paddingTop: "var(--sat)",
        paddingLeft: "var(--sal)",
        paddingRight: "var(--sar)",
      }}
    >
      <header className="flex h-[52px] shrink-0 items-center border-b border-border/60">
        <div className="hidden h-full shrink-0 items-center px-4 md:flex" style={{ width: sidebarWidth }}>
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
          <div className="hidden shrink-0 text-12 text-text-tertiary sm:block">
            assistant
          </div>
          <span className="hidden text-12 text-text-tertiary sm:block">
            /
          </span>
          <div className="min-w-0 flex-1 truncate text-12 text-muted-foreground">
            {onRenameTitle ? <ThreadTitle key={titleKey} title={title} onRename={onRenameTitle} /> : title}
          </div>
          {headerActions}
          <ThemeToggle className="shrink-0" />
          {profileMenu}
        </div>
      </header>

      <div className="flex min-h-0 flex-1 overflow-hidden">
        <aside
          className="relative hidden shrink-0 border-r border-border/60 md:block"
          style={{ width: sidebarWidth }}
        >
          <div className="h-full overflow-hidden">{sidebar}</div>
          <SidebarResizeHandle
            sidebar="assistant"
            label="Resize sidebar"
            onPreview={setPreviewWidth}
          />
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
                const target = event.target as HTMLElement;
                // React bubbles clicks from portaled layers (dialogs and menus
                // opened from the drawer) through here; those are not drawer
                // navigation, and closing would unmount them mid-action (a
                // dialog's submit would never fire).
                if (!event.currentTarget.contains(target)) return;
                // Menu triggers open something anchored inside the drawer --
                // closing it would unmount the anchor out from under them.
                if (target.closest("[data-keep-drawer-open]")) return;
                if (target.closest("a,button")) {
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
    </OverlayLayer>
  );
}
