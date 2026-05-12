import { Suspense, useState, useCallback, createContext, useContext } from "react";
import { Outlet, Link, useNavigate, useRouterState } from "@tanstack/react-router";
import { Sidebar } from "@/components/dashboard/sidebar";
import { CommandPalette } from "@/components/navigation/command-palette";
import { AmbientStatusLine } from "@/components/chrome/ambient-status-line";
import { useAuthStore } from "@/stores/auth-store";
import { useLogout } from "@/hooks/use-auth";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { ChevronRight, Menu, Search, User, Github } from "lucide-react";

type RightPanelContextType = {
  setRightPanel: (node: React.ReactNode) => void;
};

const RightPanelContext = createContext<RightPanelContextType>({
  setRightPanel: () => {},
});

export function useRightPanel() {
  return useContext(RightPanelContext);
}

export function DashboardLayout() {
  const [commandOpen, setCommandOpen] = useState(false);
  const [mobileNavOpen, setMobileNavOpen] = useState(false);
  const [rightPanel, setRightPanel] = useState<React.ReactNode>(null);

  const closeMobileNav = useCallback(() => setMobileNavOpen(false), []);

  return (
    <RightPanelContext.Provider value={{ setRightPanel }}>
      <div
        className="flex flex-col h-dvh overflow-hidden bg-background"
        style={{
          paddingTop: "var(--sat)",
          paddingLeft: "var(--sal)",
          paddingRight: "var(--sar)",
        }}
      >
        <AmbientStatusLine />

        {/* Top bar — full width */}
        <TopBar
          onSearch={() => setCommandOpen(true)}
          onMobileMenu={() => setMobileNavOpen(true)}
        />

        {/* Below top bar: sidebar + content */}
        <div className="flex flex-1 min-h-0 overflow-hidden">
          {/* Left sidebar */}
          <div className="hidden md:flex shrink-0">
            <Sidebar />
          </div>

          {/* Main content */}
          <main
            className="flex-1 min-w-0 overflow-x-hidden overflow-y-auto overscroll-contain px-6 pt-6 md:px-8 lg:px-10"
            style={{ paddingBottom: "max(2rem, var(--sab))" }}
          >
            <div className="w-full">
              <Suspense>
                <Outlet />
              </Suspense>
            </div>
          </main>

          {/* Right panel — stacked cards */}
          {rightPanel && (
            <aside className="hidden lg:flex shrink-0 w-[280px] flex-col overflow-y-auto px-3 pt-6 pb-6">
              <div className="flex flex-col gap-3">
                {rightPanel}
              </div>
            </aside>
          )}
        </div>

        {/* Mobile drawer */}
        {mobileNavOpen && (
          <div className="fixed inset-0 z-[80] flex md:hidden">
            <div
              className="fixed inset-0 bg-black/60 backdrop-blur-sm"
              onClick={closeMobileNav}
              role="button"
              tabIndex={-1}
              aria-label="Close navigation"
              onKeyDown={(e) => { if (e.key === "Escape") closeMobileNav(); }}
            />
            <div className="relative z-10 animate-in slide-in-from-left duration-200">
              <Sidebar onNavigate={closeMobileNav} />
            </div>
          </div>
        )}

        <CommandPalette open={commandOpen} onOpenChange={setCommandOpen} />
      </div>
    </RightPanelContext.Provider>
  );
}

const SIDEBAR_ITEMS: Record<string, string> = {
  "/dashboard": "Dashboard",
  "/keys": "AI Services",
  "/orgs": "Organizations",
  "/nodes": "Nodes",
  "/channel-bots": "Channel Bots",
  "/settings": "Settings",
  "/settings/consents": "Authorized Apps",
  "/settings/authorizations": "Authorizations",
  "/guide": "Guide",
  "/approvals/settings": "Notifications",
  "/approvals/history": "Approval History",
  "/approvals/grants": "Active Grants",
  "/developer/apps": "Developer Apps",
  "/ai-setup": "AI Setup",
  "/integration-guide": "Integration Guide",
  "/admin/users": "Users",
  "/admin/audit-log": "Audit Log",
  "/admin/service-accounts": "Service Accounts",
  "/admin/roles": "Roles",
  "/admin/groups": "Groups",
  "/admin/invite-codes": "Invite Codes",
  "/admin/services": "Services",
  "/admin/providers": "Providers",
  "/design-system": "Design System",
};

const SEGMENT_LABELS: Record<string, string> = {
  "api-key": "Agent Keys",
  "cli-auth": "CLI Auth",
};

const SKIP_SEGMENTS = new Set(["conversations"]);

const UUID_RE = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

const ROUTE_LINK_OVERRIDES: Record<string, string> = {
  "/keys/api-key": "/keys?tab=nyxid",
};

function TopBarBreadcrumbs() {
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  const segments = pathname.split("/").filter(Boolean);

  if (segments.length === 0) return null;

  const accPaths: string[] = [];
  let acc = "";
  for (const segment of segments) {
    acc += `/${segment}`;
    accPaths.push(acc);
  }

  const crumbs: { label: string; to?: string }[] = [];
  for (const [i, segment] of segments.entries()) {
    const segPath = accPaths[i];
    if (UUID_RE.test(segment)) continue;
    if (SKIP_SEGMENTS.has(segment)) continue;

    const laterIsSidebarItem = accPaths.slice(i + 1).some((p) => p in SIDEBAR_ITEMS);
    if (laterIsSidebarItem) continue;

    const label = SIDEBAR_ITEMS[segPath] ?? SEGMENT_LABELS[segment] ?? segment;
    const isLast = i === segments.length - 1;
    const linkTo = isLast ? undefined : (ROUTE_LINK_OVERRIDES[segPath] ?? segPath);
    crumbs.push({ label, to: linkTo });
  }

  return (
    <nav aria-label="Breadcrumb" className="hidden md:flex items-center gap-1 text-[12px] min-w-0">
      {crumbs.map((crumb, i) => (
        <div key={crumb.label + String(i)} className="flex items-center gap-1 min-w-0">
          {i > 0 && <ChevronRight className="h-3 w-3 shrink-0 text-text-tertiary/60" />}
          {crumb.to ? (
            <Link to={crumb.to} className="text-text-tertiary truncate transition-colors duration-200 hover:text-foreground">
              {crumb.label}
            </Link>
          ) : (
            <span className="text-muted-foreground truncate">{crumb.label}</span>
          )}
        </div>
      ))}
    </nav>
  );
}

function TopBar({
  onSearch,
  onMobileMenu,
}: {
  readonly onSearch: () => void;
  readonly onMobileMenu: () => void;
}) {
  const user = useAuthStore((s) => s.user);
  const navigate = useNavigate();
  const logoutMutation = useLogout();
  async function handleLogout() {
    await logoutMutation.mutateAsync();
    void navigate({ to: "/login" as string });
  }

  return (
    <header className="flex items-center shrink-0 h-[52px] border-b border-border/60">
      {/* Mobile hamburger */}
      <button
        type="button"
        onClick={onMobileMenu}
        className="flex h-8 w-8 items-center justify-center rounded-lg text-muted-foreground md:hidden ml-4 mr-3"
        aria-label="Open menu"
      >
        <Menu className="h-4 w-4" />
      </button>

      {/* Logo zone — matches sidebar width, icon pinned left */}
      <Link
        to="/dashboard"
        className="hidden md:flex items-center shrink-0 justify-center w-[52px] transition-[width] duration-300 ease-in-out"
        style={{ width: "var(--sidebar-width, 52px)", justifyContent: "start", paddingLeft: "16px" }}
      >
        <img
          src="/nyxid-coloured-icon.svg"
          alt="NyxID"
          className="h-5 w-5 shrink-0"
        />
      </Link>

      {/* Content zone — breadcrumbs aligned with main content padding */}
      <div className="flex flex-1 items-center min-w-0 px-6 md:px-8 lg:px-10">
        <TopBarBreadcrumbs />

        <div className="flex-1" />

        {/* Right actions */}
        <div className="flex items-center gap-2">
        {/* Search */}
        <button
          type="button"
          onClick={onSearch}
          className="flex h-8 items-center gap-2 rounded-lg border border-white/[0.08] px-3 text-[12px] text-text-tertiary transition-colors duration-300 hover:border-white/[0.15] hover:text-muted-foreground"
        >
          <Search className="h-[14px] w-[14px]" />
          <span className="hidden sm:inline">Search...</span>
          <kbd className="ml-1 hidden h-[18px] w-[18px] items-center justify-center rounded-[4px] border border-white/[0.08] bg-white/[0.04] text-[10px] text-text-tertiary sm:flex">/</kbd>
        </button>

        {/* Profile */}
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <button
              type="button"
              className="flex h-8 w-8 items-center justify-center rounded-lg border border-white/[0.08] text-text-tertiary transition-colors duration-300 hover:border-white/[0.15] hover:text-muted-foreground focus-visible:outline-none"
              aria-label="User menu"
            >
              <User className="h-[14px] w-[14px]" />
            </button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" className="w-48 p-2">
            <div className="px-2 py-1.5">
              <p className="text-[12px] font-medium text-foreground">{user?.display_name ?? "User"}</p>
              <p className="text-[11px] text-text-tertiary">{user?.email ?? ""}</p>
            </div>
            <DropdownMenuItem
              onClick={() => void navigate({ to: "/settings" as string })}
              className="rounded-md text-[12px]"
            >
              Settings
            </DropdownMenuItem>
            <DropdownMenuItem
              onClick={() => void handleLogout()}
              className="rounded-md text-[12px] text-destructive focus:text-destructive"
            >
              Log out
            </DropdownMenuItem>
          </DropdownMenuContent>
        </DropdownMenu>

        {/* GitHub */}
        <a
          href="https://github.com/ChronoAIProject"
          target="_blank"
          rel="noopener noreferrer"
          className="flex h-8 items-center gap-1.5 rounded-lg border border-white/[0.08] px-3 text-[12px] text-text-tertiary transition-colors duration-300 hover:border-white/[0.15] hover:text-muted-foreground"
        >
          <span className="flex h-[18px] w-[18px] items-center justify-center rounded-[4px] border border-white/[0.08] bg-white/[0.04]">
            <Github className="h-3 w-3" />
          </span>
          <span className="hidden sm:inline">GitHub</span>
        </a>
        </div>
      </div>
    </header>
  );
}
