import { Suspense, useState, useEffect, useCallback, useMemo, createContext, useContext } from "react";
import { Outlet, Link, useNavigate, useRouterState } from "@tanstack/react-router";
import { Sidebar, MAIN_NAV, APPROVALS_NAV, DEVELOPER_NAV, ADMIN_NAV, isNavActive } from "@/components/dashboard/sidebar";
import { hasAdminRead } from "@/types/api";
import {
  CommandPalette,
  ALL_ITEMS as SEARCH_ITEMS,
  type CommandItem,
} from "@/components/navigation/command-palette";
import { AmbientStatusLine } from "@/components/chrome/ambient-status-line";
import { useAuthStore } from "@/stores/auth-store";
import { useLogout } from "@/hooks/use-auth";
import { useShouldShowOnboarding } from "@/hooks/use-onboarding";
import { OnboardingTakeover } from "@/components/dashboard/onboarding-takeover";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { ChevronLeft, ChevronRight, LogOut, Menu, Search, Settings, User, Github, X } from "lucide-react";

type RightPanelContextType = {
  setRightPanel: (node: React.ReactNode) => void;
};

const RightPanelContext = createContext<RightPanelContextType>({
  setRightPanel: () => {},
});

export function useRightPanel() {
  return useContext(RightPanelContext);
}

type BreadcrumbLabelContextType = {
  label: string | null;
  setLabel: (label: string | null) => void;
};

const BreadcrumbLabelContext = createContext<BreadcrumbLabelContextType>({
  label: null,
  setLabel: () => {},
});

export function useBreadcrumbLabel(label: string | undefined | null) {
  const { setLabel } = useContext(BreadcrumbLabelContext);
  const stableLabel = label ?? null;
  useEffect(() => {
    setLabel(stableLabel);
    return () => setLabel(null);
  }, [stableLabel, setLabel]);
}

export function DashboardLayout() {
  const [commandOpen, setCommandOpen] = useState(false);
  const [mobileNavState, setMobileNavState] = useState<"closed" | "open" | "closing">("closed");
  const [rightPanel, setRightPanel] = useState<React.ReactNode>(null);
  const [breadcrumbLabel, setBreadcrumbLabel] = useState<string | null>(null);

  const closeMobileNav = useCallback(() => setMobileNavState("closing"), []);

  // First-run gate: until the user finishes the onboarding wizard, render it
  // in place of the dashboard chrome. No separate route — the wizard wraps
  // over the dashboard. Gated on auth / `GET /users/me` settling so we never
  // flash the wrong thing.
  const onboarding = useShouldShowOnboarding();
  if (onboarding.status === "loading") return null;
  if (onboarding.status === "show") return <OnboardingTakeover />;

  return (
    <RightPanelContext.Provider value={{ setRightPanel }}>
    <BreadcrumbLabelContext.Provider value={{ label: breadcrumbLabel, setLabel: setBreadcrumbLabel }}>
      <div
        className="flex flex-col h-dvh overflow-hidden bg-background"
        style={{
          paddingTop: "var(--sat)",
          paddingLeft: "var(--sal)",
          paddingRight: "var(--sar)",
        }}
      >
        <AmbientStatusLine />

        <TopBar
          onSearch={() => setCommandOpen(true)}
          onMobileMenu={() => setMobileNavState("open")}
        />

        <div className="flex flex-1 min-h-0 overflow-hidden">
          <div className="hidden md:flex shrink-0">
            <Sidebar />
          </div>

          <main
            className="flex-1 min-w-0 overflow-x-hidden overflow-y-auto overscroll-contain px-4 pt-4 sm:px-6 sm:pt-6 md:px-8 lg:px-10"
            style={{ paddingBottom: "max(2rem, var(--sab))" }}
          >
            <div className="w-full">
              <Suspense>
                <Outlet />
              </Suspense>
            </div>
          </main>

          {rightPanel && (
            <aside className="hidden lg:flex shrink-0 w-[280px] flex-col overflow-y-auto px-3 pt-6 pb-6">
              <div className="flex flex-col gap-3">
                {rightPanel}
              </div>
            </aside>
          )}
        </div>

        {mobileNavState !== "closed" && (
          <MobileNav
            isClosing={mobileNavState === "closing"}
            onClose={closeMobileNav}
            onAnimationEnd={() => { if (mobileNavState === "closing") setMobileNavState("closed"); }}
          />
        )}

        <CommandPalette open={commandOpen} onOpenChange={setCommandOpen} />
      </div>
    </BreadcrumbLabelContext.Provider>
    </RightPanelContext.Provider>
  );
}

const SIDEBAR_ITEMS: Record<string, string> = {
  "/dashboard": "Dashboard",
  "/keys": "Services & Credentials",
  "/orgs": "Organizations",
  "/nodes": "Credential Nodes",
  "/channel-bots": "Channel Bots",
  "/settings": "Account Settings",
  "/settings/consents": "Access & Authorizations",
  "/guide": "Setup Guide",
  "/approvals/settings": "Notification Settings",
  "/approvals/history": "Approval History",
  "/approvals/grants": "Active Grants",
  "/developer/apps": "Developer Apps",
  "/ai-setup": "AI Setup Guide",
  "/integration-guide": "Integration & SDK Guide",
  "/admin/users": "Users",
  "/admin/audit-log": "Audit Log",
  "/admin/service-accounts": "Service Accounts",
  "/admin/roles": "Roles",
  "/admin/groups": "Groups",
  "/admin/invite-codes": "Invite Codes",
  "/admin/nodes": "Nodes",
  "/admin/services": "Services",
  "/admin/providers": "Providers",
  "/design-system": "Design System",
};

const SEGMENT_LABELS: Record<string, string> = {
  "cli-auth": "CLI Auth",
};

const SKIP_BREADCRUMB_SEGMENTS = new Set(["api-key"]);

const SKIP_SEGMENTS = new Set(["conversations"]);

const UUID_RE = /^[0-9a-f]{8}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{4}-[0-9a-f]{12}$/i;

const ROUTE_LINK_OVERRIDES: Record<string, string> = {};

const PARENT_LINK_OVERRIDES: Record<string, string> = {
  "api-key": "/keys?tab=nyxid",
};


function TopBarBreadcrumbs() {
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  const { label: detailLabel } = useContext(BreadcrumbLabelContext);
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
    const segPath = accPaths[i]!;
    const isLast = i === segments.length - 1;
    if (UUID_RE.test(segment)) {
      if (isLast && detailLabel) {
        crumbs.push({ label: detailLabel });
      }
      continue;
    }
    if (SKIP_SEGMENTS.has(segment)) continue;
    if (SKIP_BREADCRUMB_SEGMENTS.has(segment)) {
      const override = PARENT_LINK_OVERRIDES[segment];
      const last = crumbs[crumbs.length - 1];
      if (override && last) {
        last.to = override;
      }
      continue;
    }

    const laterIsSidebarItem = accPaths.slice(i + 1).some((p) => p in SIDEBAR_ITEMS);
    if (laterIsSidebarItem) continue;

    const label = SIDEBAR_ITEMS[segPath] ?? SEGMENT_LABELS[segment] ?? segment;
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
  readonly onMobileMenu?: () => void;
}) {
  const user = useAuthStore((s) => s.user);
  const navigate = useNavigate();
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  const logoutMutation = useLogout();
  async function handleLogout() {
    await logoutMutation.mutateAsync();
    void navigate({ to: "/login" as string });
  }

  const ROOT_PATHS = new Set([
    "/dashboard", "/keys", "/orgs", "/nodes", "/channel-bots",
    "/settings", "/guide", "/approvals/settings", "/approvals/history",
    "/approvals/grants", "/developer/apps", "/ai-setup", "/integration-guide",
  ]);
  const showBack = !ROOT_PATHS.has(pathname);

  return (
    <header className="flex items-center shrink-0 h-[52px] border-b border-border/60">
      {/* Mobile: back + logo left */}
      <div className="flex items-center pl-2 gap-1 md:hidden">
        {showBack ? (
          <button
            type="button"
            onClick={() => window.history.back()}
            className="flex h-8 w-8 items-center justify-center rounded-lg text-muted-foreground"
            aria-label="Go back"
          >
            <ChevronLeft className="h-5 w-5" />
          </button>
        ) : (
          <Link to="/dashboard" className="pl-2">
            <img src="/nyxid-coloured-icon.svg" alt="NyxID" className="h-5 w-5" />
          </Link>
        )}
      </div>

      {/* Desktop: logo zone — matches sidebar width */}
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

      {/* Content zone */}
      <div className="flex flex-1 items-center min-w-0 px-4 md:px-8 lg:px-10">
        <TopBarBreadcrumbs />

        <div className="flex-1" />

        {/* Right actions */}
        <div className="flex items-center gap-2">
        {/* Search — desktop only */}
        <button
          type="button"
          onClick={onSearch}
          className="hidden md:flex h-8 items-center gap-2 rounded-lg border border-white/[0.08] px-3 text-[12px] text-text-tertiary transition-colors duration-300 hover:border-white/[0.15] hover:text-muted-foreground"
        >
          <Search className="h-[14px] w-[14px]" />
          <span>Search...</span>
          <kbd className="ml-1 flex h-[18px] w-[18px] items-center justify-center rounded-[4px] border border-white/[0.08] bg-white/[0.04] text-[10px] text-text-tertiary">/</kbd>
        </button>

        {/* Profile — desktop only (mobile has it in the menu) */}
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <button
              type="button"
              className="hidden md:flex h-8 w-8 items-center justify-center rounded-lg border border-white/[0.08] text-text-tertiary transition-colors duration-300 hover:border-white/[0.15] hover:text-muted-foreground focus-visible:outline-none"
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
              onClick={() => void navigate({ to: "/settings" })}
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

        {/* GitHub — desktop only */}
        <a
          href="https://github.com/ChronoAIProject"
          target="_blank"
          rel="noopener noreferrer"
          className="hidden md:flex h-8 items-center gap-1.5 rounded-lg border border-white/[0.08] px-3 text-[12px] text-text-tertiary transition-colors duration-300 hover:border-white/[0.15] hover:text-muted-foreground"
        >
          <span className="flex h-[18px] w-[18px] items-center justify-center rounded-[4px] border border-white/[0.08] bg-white/[0.04]">
            <Github className="h-3 w-3" />
          </span>
          <span>GitHub</span>
        </a>

        {/* Mobile: profile */}
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <button
              type="button"
              className="flex md:hidden h-8 w-8 items-center justify-center rounded-lg border border-white/[0.08] text-text-tertiary"
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
              onClick={() => void navigate({ to: "/settings" })}
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

        {/* Mobile: GitHub */}
        <a
          href="https://github.com/ChronoAIProject"
          target="_blank"
          rel="noopener noreferrer"
          className="flex md:hidden h-8 w-8 items-center justify-center rounded-lg border border-white/[0.08] text-text-tertiary transition-colors duration-300 hover:border-white/[0.15] hover:text-muted-foreground"
          aria-label="GitHub"
        >
          <Github className="h-[14px] w-[14px]" />
        </a>

        {onMobileMenu && (
        <button
          type="button"
          onClick={onMobileMenu}
          className="flex md:hidden h-8 w-8 items-center justify-center rounded-lg text-muted-foreground"
          aria-label="Open menu"
        >
          <Menu className="h-4 w-4" />
        </button>
        )}
        </div>
      </div>
    </header>
  );
}

function MobileNavItem({
  item,
  active,
  onClick,
}: {
  readonly item: { to: string; icon: React.ComponentType<{ className?: string }>; label: string };
  readonly active: boolean;
  readonly onClick: () => void;
}) {
  return (
    <Link
      to={item.to}
      onClick={onClick}
      className={`flex items-center gap-3 rounded-xl px-4 py-3 text-[14px] transition-colors ${
        active
          ? "bg-white/[0.06] font-medium text-foreground"
          : "text-muted-foreground active:bg-white/[0.04]"
      }`}
    >
      <item.icon
        className={`h-[18px] w-[18px] shrink-0 ${
          active ? "text-nyx-secondary-400" : "text-text-tertiary"
        }`}
      />
      {item.label}
    </Link>
  );
}

function MobileNav({
  isClosing,
  onClose,
  onAnimationEnd,
}: {
  readonly isClosing: boolean;
  readonly onClose: () => void;
  readonly onAnimationEnd: () => void;
}) {
  const user = useAuthStore((s) => s.user);
  const navigate = useNavigate();
  const logoutMutation = useLogout();
  const pathname = useRouterState({ select: (s) => s.location.pathname });
  const isAdmin = hasAdminRead(user);
  const allItems = [...MAIN_NAV, ...APPROVALS_NAV, ...DEVELOPER_NAV, ...(isAdmin ? ADMIN_NAV : [])];
  const [searchQuery, setSearchQuery] = useState("");

  const searchResults = useMemo(() => {
    if (!searchQuery.trim()) return null;
    const q = searchQuery.toLowerCase();
    return SEARCH_ITEMS.filter(
      (item) =>
        item.label.toLowerCase().includes(q) ||
        (item.to?.toLowerCase().includes(q) ?? false),
    );
  }, [searchQuery]);

  async function handleLogout() {
    onClose();
    await logoutMutation.mutateAsync();
    void navigate({ to: "/login" as string });
  }

  function handleSearchSelect(item: CommandItem) {
    onClose();
    if (item.onSelect) {
      item.onSelect();
      return;
    }
    if (item.to) {
      // Mirror the command palette: navigate with the structured `search`
      // so deep-link actions like `?action=add-service` survive the
      // TanStack search-param validator and the keys page can auto-open
      // the matching dialog.
      void navigate({
        to: item.to as never,
        search: (item.search ?? {}) as never,
      });
    }
  }

  return (
    <div
      className={`fixed inset-0 z-[80] flex flex-col bg-background md:hidden duration-200 ${
        isClosing
          ? "animate-out slide-out-to-bottom fill-mode-forwards"
          : "animate-in slide-in-from-bottom"
      }`}
      onAnimationEnd={onAnimationEnd}
    >
      {/* Header */}
      <div className="flex items-center justify-between shrink-0 h-[56px] px-5" style={{ paddingTop: "var(--sat)" }}>
        <div className="flex items-center gap-3">
          <img src="/nyxid-coloured-icon.svg" alt="NyxID" className="h-5 w-5" />
          <span className="text-[14px] font-semibold text-foreground">NyxID</span>
        </div>
        <button
          type="button"
          onClick={onClose}
          className="flex h-8 w-8 items-center justify-center rounded-lg text-muted-foreground"
          aria-label="Close menu"
        >
          <X className="h-4 w-4" />
        </button>
      </div>

      {/* Search input */}
      <div className="px-5 pb-3">
        <div className="flex h-10 items-center gap-3 rounded-xl border border-white/[0.08] bg-white/[0.02] px-4">
          <Search className="h-4 w-4 shrink-0 text-text-tertiary" />
          <input
            type="text"
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
            placeholder="Search..."
            className="flex-1 bg-transparent text-[13px] text-foreground placeholder:text-text-tertiary outline-none"
          />
          {searchQuery && (
            <button type="button" onClick={() => setSearchQuery("")} className="text-text-tertiary">
              <X className="h-3.5 w-3.5" />
            </button>
          )}
        </div>
      </div>

      {/* Navigation / Search results */}
      <nav className="flex-1 overflow-y-auto px-3 pb-4">
        {searchResults !== null ? (
          searchResults.length > 0 ? (
            <div className="flex flex-col gap-0.5">
              {searchResults.map((item) => (
                <button
                  key={`${item.to ?? "action"}-${item.label}`}
                  type="button"
                  onClick={() => handleSearchSelect(item)}
                  className="flex items-center gap-3 rounded-xl px-4 py-3 text-[14px] text-muted-foreground active:bg-white/[0.04]"
                >
                  <item.icon className="h-[18px] w-[18px] shrink-0 text-text-tertiary" />
                  <span className="flex-1 text-left">{item.label}</span>
                  {item.group === "action" && (
                    <span className="text-[10px] font-semibold uppercase tracking-[1.5px] text-text-tertiary">Action</span>
                  )}
                </button>
              ))}
            </div>
          ) : (
            <div className="py-8 text-center text-[13px] text-text-tertiary">
              No results for &ldquo;{searchQuery}&rdquo;
            </div>
          )
        ) : (
          <>
            <div className="flex flex-col gap-0.5">
              {MAIN_NAV.map((item) => (
                <MobileNavItem
                  key={item.to}
                  item={item}
                  active={isNavActive(item.to, pathname, allItems)}
                  onClick={onClose}
                />
              ))}
            </div>

            <div className="px-4 my-3">
              <span className="text-[10px] font-medium uppercase tracking-[1.5px] text-text-tertiary/50">
                Approvals
              </span>
            </div>
            <div className="flex flex-col gap-0.5">
              {APPROVALS_NAV.map((item) => (
                <MobileNavItem
                  key={item.to}
                  item={item}
                  active={isNavActive(item.to, pathname, allItems)}
                  onClick={onClose}
                />
              ))}
            </div>

            <div className="px-4 my-3">
              <span className="text-[10px] font-medium uppercase tracking-[1.5px] text-text-tertiary/50">
                Developer
              </span>
            </div>
            <div className="flex flex-col gap-0.5">
              {DEVELOPER_NAV.map((item) => (
                <MobileNavItem
                  key={item.to}
                  item={item}
                  active={isNavActive(item.to, pathname, allItems)}
                  onClick={onClose}
                />
              ))}
            </div>

            {isAdmin && (
              <>
                <div className="px-4 my-3">
                  <span className="text-[10px] font-medium uppercase tracking-[1.5px] text-text-tertiary/50">
                    Admin
                  </span>
                </div>
                <div className="flex flex-col gap-0.5">
                  {ADMIN_NAV.map((item) => (
                    <MobileNavItem
                      key={item.to}
                      item={item}
                      active={isNavActive(item.to, pathname, allItems)}
                      onClick={onClose}
                    />
                  ))}
                </div>
              </>
            )}
          </>
        )}
      </nav>

      {/* Footer — user info + logout */}
      <div className="shrink-0 border-t border-border/60 px-5 py-4 space-y-2" style={{ paddingBottom: "max(1rem, var(--sab))" }}>
        <div className="flex items-center gap-3">
          <div className="flex h-8 w-8 items-center justify-center rounded-lg border border-white/[0.08] bg-white/[0.04]">
            <User className="h-[14px] w-[14px] text-text-tertiary" />
          </div>
          <div className="min-w-0 flex-1">
            <p className="text-[13px] font-medium text-foreground truncate">{user?.display_name ?? "User"}</p>
            <p className="text-[11px] text-text-tertiary truncate">{user?.email ?? ""}</p>
          </div>
        </div>
        <div className="flex gap-2">
          <button
            type="button"
            onClick={() => { onClose(); void navigate({ to: "/settings" }); }}
            className="flex flex-1 items-center justify-center gap-2 rounded-xl border border-white/[0.08] bg-white/[0.02] py-2.5 text-[12px] text-muted-foreground active:bg-white/[0.04]"
          >
            <Settings className="h-3.5 w-3.5" />
            Settings
          </button>
          <button
            type="button"
            onClick={() => void handleLogout()}
            className="flex flex-1 items-center justify-center gap-2 rounded-xl border border-white/[0.08] bg-white/[0.02] py-2.5 text-[12px] text-destructive active:bg-white/[0.04]"
          >
            <LogOut className="h-3.5 w-3.5" />
            Log out
          </button>
        </div>
      </div>
    </div>
  );
}
