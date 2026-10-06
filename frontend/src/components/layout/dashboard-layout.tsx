import { Suspense, useState, useEffect, useCallback, useMemo, createContext, useContext } from "react";
import { Outlet, Link, useNavigate, useRouterState } from "@tanstack/react-router";
import { Sidebar, AssistantNavEntry, APPROVALS_NAV, DEVELOPER_NAV, ADMIN_NAV, getVisibleMainNav, isNavActive } from "@/components/dashboard/sidebar";
import { canAdminWrite, hasAdminRead } from "@/types/api";
import {
  CommandPalette,
  ALL_ITEMS as SEARCH_ITEMS,
  type CommandItem,
} from "@/components/navigation/command-palette";
import { AmbientStatusLine } from "@/components/chrome/ambient-status-line";
import { useAuthStore } from "@/stores/auth-store";
import { useLogout, useUser } from "@/hooks/use-auth";
import { useShouldShowOnboarding } from "@/hooks/use-onboarding";
import { useApplyTheme } from "@/hooks/use-theme";
import { normalizeScreenKey } from "@/lib/assistant/screen-context";
import { NyxidLogo } from "@/components/brand/nyxid-logo";
import { OnboardingTakeover } from "@/components/dashboard/onboarding-takeover";
import { ThemeToggle } from "@/components/dashboard/theme-toggle";
import { BreadcrumbLabelContext } from "./breadcrumb-context";
import { buildStudioBreadcrumbs } from "@/lib/studio-breadcrumbs";
import { StudioBreadcrumbTrail } from "./studio-breadcrumb-trail";
export { useBreadcrumbLabel } from "./breadcrumb-context";
import { useAssistantContextStore } from "@/stores/assistant-context-store";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { ChevronLeft, LogOut, Menu, Search, Settings, User, Github, X } from "lucide-react";

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
  // Keep an active /users/me observer for every dashboard session: its
  // queryFn syncs the auth store, so server-side capability changes (feature
  // flags flipped by an admin, mutation-triggered ["user"] invalidations)
  // reach flag-gated UI like the sidebar without a hard reload.
  const userQuery = useUser();
  const [commandOpen, setCommandOpen] = useState(false);
  const [mobileNavState, setMobileNavState] = useState<"closed" | "open" | "closing">("closed");
  const [rightPanel, setRightPanel] = useState<React.ReactNode>(null);
  const [breadcrumbLabels, setBreadcrumbLabels] = useState<Record<string, string>>({});
  const [breadcrumbSections, setBreadcrumbSections] = useState<Record<string, string>>({});
  const setBreadcrumbSection = useCallback((path: string, section: string | null) => {
    setBreadcrumbSections((current) => {
      if (current[path] === section || (!section && !(path in current))) return current;
      const next = { ...current };
      if (section) next[path] = section;
      else delete next[path];
      return next;
    });
  }, []);
  const setBreadcrumbLabel = useCallback((path: string, label: string | null) => {
    setBreadcrumbLabels((current) => {
      if (current[path] === label || (!label && !(path in current))) return current;
      const next = { ...current };
      if (label) next[path] = label;
      else delete next[path];
      return next;
    });
  }, []);
  const pathname = useRouterState({ select: (s) => s.location.pathname });

  // Applies the resolved theme class to <html> on mount, reverts to dark on
  // unmount. Must run before the onboarding early-returns below to satisfy
  // rules-of-hooks.
  useApplyTheme();

  useEffect(() => {
    document.title = `nyxid - ${sectionTitleFor(pathname)}`;
  }, [pathname]);

  useEffect(() => {
    const screenKey = normalizeScreenKey(pathname);
    const userId = userQuery.data?.id ?? useAuthStore.getState().user?.id;
    if (userId && screenKey) {
      useAssistantContextStore.getState().recordScreen(userId, screenKey);
    }
  }, [pathname, userQuery.data?.id]);

  const closeMobileNav = useCallback(() => setMobileNavState("closing"), []);

  // First-run gate: until the user finishes the onboarding wizard, render it
  // in place of the dashboard chrome. No separate route — the wizard wraps
  // over the dashboard. Gated on auth / `GET /users/me` settling so we never
  // flash the wrong thing.
  const onboarding = useShouldShowOnboarding();
  // Shared channel onboarding must stay reachable through setup and bot routing.
  const isChannelBotRoute = pathname === "/channel-bots" || pathname.startsWith("/channel-bots/");
  if (onboarding.status === "loading") return null;
  if (onboarding.status === "show" && !isChannelBotRoute) return <OnboardingTakeover />;

  return (
    <RightPanelContext.Provider value={{ setRightPanel }}>
    <BreadcrumbLabelContext.Provider value={{ pathname, labels: breadcrumbLabels, setLabel: setBreadcrumbLabel, sections: breadcrumbSections, setSection: setBreadcrumbSection }}>
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

const SECTION_TITLES: Record<string, string> = {
  dashboard: "dashboard",
  billing: "billing & usage",
  keys: "ai services",
  orgs: "org",
  nodes: "nodes",
  "channel-bots": "channel bots",
  settings: "settings",
  guide: "guide",
  approvals: "approvals",
  developer: "developer apps",
  "ai-setup": "ai setup",
  "integration-guide": "integration guide",
  admin: "admin",
  "design-system": "design system",
};

function sectionTitleFor(pathname: string): string {
  const first = pathname.split("/").filter(Boolean)[0] ?? "";
  return SECTION_TITLES[first] ?? "dashboard";
}

function TopBarBreadcrumbs() {
  const { pathname, labels, sections } = useContext(BreadcrumbLabelContext);
  const search = useRouterState({ select: (s) => s.location.search });
  const crumbs = buildStudioBreadcrumbs(pathname, labels, search, sections?.[pathname]);
  return <StudioBreadcrumbTrail crumbs={crumbs} />;
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
            <NyxidLogo className="h-6 w-auto" />
          </Link>
        )}
      </div>

      {/* Desktop: logo zone — fits the full brand mark; breadcrumb starts after */}
      <Link
        to="/dashboard"
        className="hidden md:flex items-center shrink-0 pl-4 pr-2"
      >
        <NyxidLogo className="h-5 w-auto" />
      </Link>

      {/* Content zone */}
      <div className="flex flex-1 items-center min-w-0 px-4 md:px-8 lg:px-10">
        <TopBarBreadcrumbs />

        <div className="flex-1" />

        {/* Right actions */}
        <div className="flex items-center gap-2">
        {/* Theme toggle — light/dark, all breakpoints */}
        <ThemeToggle />

        {/* Search — desktop only */}
        <button
          type="button"
          onClick={onSearch}
          className="hidden md:flex h-8 items-center gap-2 rounded-lg border border-hairline px-3 text-12 text-text-tertiary transition-colors duration-300 hover:border-hairline-strong hover:text-muted-foreground"
        >
          <Search className="h-[14px] w-[14px]" />
          <span>Search...</span>
          <kbd className="ml-1 flex h-[18px] w-[18px] items-center justify-center rounded-[4px] border border-hairline bg-overlay-strong text-10 text-text-tertiary">/</kbd>
        </button>

        {/* Profile — desktop only (mobile has it in the menu) */}
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <button
              type="button"
              className="hidden md:flex h-8 w-8 items-center justify-center rounded-lg border border-hairline text-text-tertiary transition-colors duration-300 hover:border-hairline-strong hover:text-muted-foreground focus-visible:outline-none"
              aria-label="User menu"
            >
              <User className="h-[14px] w-[14px]" />
            </button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" className="w-48 p-2">
            <div className="px-2 py-1.5">
              <p className="text-12 font-medium text-foreground">{user?.display_name ?? "User"}</p>
              <p className="text-11 text-text-tertiary">{user?.email ?? ""}</p>
            </div>
            <DropdownMenuItem
              onClick={() => void navigate({ to: "/settings" })}
              className="rounded-md text-12"
            >
              Settings
            </DropdownMenuItem>
            <DropdownMenuItem
              onClick={() => void handleLogout()}
              className="rounded-md text-12 text-destructive focus:text-destructive"
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
          className="hidden md:flex h-8 items-center gap-1.5 rounded-lg border border-hairline px-3 text-12 text-text-tertiary transition-colors duration-300 hover:border-hairline-strong hover:text-muted-foreground"
        >
          <span className="flex h-[18px] w-[18px] items-center justify-center rounded-[4px] border border-hairline bg-overlay-strong">
            <Github className="h-3 w-3" />
          </span>
          <span>GitHub</span>
        </a>

        {/* Mobile: profile */}
        <DropdownMenu>
          <DropdownMenuTrigger asChild>
            <button
              type="button"
              className="flex md:hidden h-8 w-8 items-center justify-center rounded-lg border border-hairline text-text-tertiary"
              aria-label="User menu"
            >
              <User className="h-[14px] w-[14px]" />
            </button>
          </DropdownMenuTrigger>
          <DropdownMenuContent align="end" className="w-48 p-2">
            <div className="px-2 py-1.5">
              <p className="text-12 font-medium text-foreground">{user?.display_name ?? "User"}</p>
              <p className="text-11 text-text-tertiary">{user?.email ?? ""}</p>
            </div>
            <DropdownMenuItem
              onClick={() => void navigate({ to: "/settings" })}
              className="rounded-md text-12"
            >
              Settings
            </DropdownMenuItem>
            <DropdownMenuItem
              onClick={() => void handleLogout()}
              className="rounded-md text-12 text-destructive focus:text-destructive"
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
          className="flex md:hidden h-8 w-8 items-center justify-center rounded-lg border border-hairline text-text-tertiary transition-colors duration-300 hover:border-hairline-strong hover:text-muted-foreground"
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
      className={`flex items-center gap-3 rounded-xl px-4 py-3 text-14 transition-colors ${
        active
          ? "bg-overlay-strong font-medium text-foreground"
          : "text-muted-foreground active:bg-overlay-strong"
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
  const mainNav = getVisibleMainNav(user);
  const allItems = [...mainNav, ...APPROVALS_NAV, ...DEVELOPER_NAV, ...(isAdmin ? ADMIN_NAV : [])];
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
        <NyxidLogo className="h-6 w-auto" />
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
        <div className="flex h-10 items-center gap-3 rounded-xl border border-hairline bg-overlay px-4">
          <Search className="h-4 w-4 shrink-0 text-text-tertiary" />
          <input
            type="text"
            value={searchQuery}
            onChange={(e) => setSearchQuery(e.target.value)}
            placeholder="Search..."
            className="flex-1 bg-transparent text-13 text-foreground placeholder:text-text-tertiary outline-none"
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
                  className="flex items-center gap-3 rounded-xl px-4 py-3 text-14 text-muted-foreground active:bg-overlay-strong"
                >
                  <item.icon className="h-[18px] w-[18px] shrink-0 text-text-tertiary" />
                  <span className="flex-1 text-left">{item.label}</span>
                  {item.group === "action" && (
                    <span className="text-10 font-semibold uppercase tracking-[1.5px] text-text-tertiary">Action</span>
                  )}
                </button>
              ))}
            </div>
          ) : (
            <div className="py-8 text-center text-13 text-text-tertiary">
              No results for &ldquo;{searchQuery}&rdquo;
            </div>
          )
        ) : (
          <>
            <AssistantNavEntry mobile onClick={onClose} />
            <div className="flex flex-col gap-0.5">
              {mainNav.map((item) => (
                <MobileNavItem
                  key={item.to}
                  item={item}
                  active={isNavActive(item.to, pathname, allItems)}
                  onClick={onClose}
                />
              ))}
            </div>

            <div className="px-4 my-3">
              <span className="text-10 font-medium uppercase tracking-[1.5px] text-text-tertiary">
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
              <span className="text-10 font-medium uppercase tracking-[1.5px] text-text-tertiary">
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
                  <span className="text-10 font-medium uppercase tracking-[1.5px] text-text-tertiary">
                    Admin
                  </span>
                </div>
                <div className="flex flex-col gap-0.5">
                  {ADMIN_NAV.filter((item) => !["/admin/platform-credentials", "/admin/upload-retention"].includes(item.to) || canAdminWrite(user)).map((item) => (
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
          <div className="flex h-8 w-8 items-center justify-center rounded-lg border border-hairline bg-overlay-strong">
            <User className="h-[14px] w-[14px] text-text-tertiary" />
          </div>
          <div className="min-w-0 flex-1">
            <p className="text-13 font-medium text-foreground truncate">{user?.display_name ?? "User"}</p>
            <p className="text-11 text-text-tertiary truncate">{user?.email ?? ""}</p>
          </div>
        </div>
        <div className="flex gap-2">
          <button
            type="button"
            onClick={() => { onClose(); void navigate({ to: "/settings" }); }}
            className="flex flex-1 items-center justify-center gap-2 rounded-xl border border-hairline bg-overlay py-2.5 text-12 text-muted-foreground active:bg-overlay-strong"
          >
            <Settings className="h-3.5 w-3.5" />
            Settings
          </button>
          <button
            type="button"
            onClick={() => void handleLogout()}
            className="flex flex-1 items-center justify-center gap-2 rounded-xl border border-hairline bg-overlay py-2.5 text-12 text-destructive active:bg-overlay-strong"
          >
            <LogOut className="h-3.5 w-3.5" />
            Log out
          </button>
        </div>
      </div>
    </div>
  );
}
