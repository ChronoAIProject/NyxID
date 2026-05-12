import { useState, useEffect, useCallback, useRef } from "react";
import { useRouterState, Link } from "@tanstack/react-router";
import {
  LayoutDashboard,
  Cable,
  HardDrive,
  Settings,
  BookOpen,
  BookMarked,
  Code,
  Bell,
  ClipboardList,
  Lock,
  Sparkles,
  Building2,
  Radio,
  KeyRound,
  Link2,
  PanelLeftClose,
  PanelLeft,
  Circle,
} from "lucide-react";
import {
  Popover,
  PopoverContent,
  PopoverTrigger,
} from "@/components/ui/popover";
import { cn } from "@/lib/utils";

type SidebarMode = "expanded" | "collapsed" | "hover";
const STORAGE_KEY = "nyxid:sidebar-mode";

const MAIN = [
  { to: "/dashboard", icon: LayoutDashboard, label: "Dashboard" },
  { to: "/keys", icon: Cable, label: "AI Services" },
  { to: "/orgs", icon: Building2, label: "Organizations" },
  { to: "/nodes", icon: HardDrive, label: "Nodes" },
  { to: "/channel-bots", icon: Radio, label: "Channel Bots" },
  { to: "/settings", icon: Settings, label: "Settings" },
  { to: "/settings/consents", icon: KeyRound, label: "Authorized Apps" },
  { to: "/settings/authorizations", icon: Link2, label: "Authorizations" },
  { to: "/guide", icon: BookOpen, label: "Guide" },
] as const;

const APPROVALS = [
  { to: "/approvals/settings", icon: Bell, label: "Notifications" },
  { to: "/approvals/history", icon: ClipboardList, label: "Approval History" },
  { to: "/approvals/grants", icon: Lock, label: "Active Grants" },
] as const;

const DEVELOPER = [
  { to: "/developer/apps", icon: Code, label: "Developer Apps" },
  { to: "/ai-setup", icon: Sparkles, label: "AI Setup" },
  { to: "/integration-guide", icon: BookMarked, label: "Integration Guide" },
] as const;

type NavItemDef = {
  readonly to: string;
  readonly icon: React.ComponentType<{ className?: string }>;
  readonly label: string;
};

function isNavActive(
  itemTo: string,
  currentPath: string,
  allItems: readonly NavItemDef[],
): boolean {
  if (itemTo === "/dashboard") return currentPath === "/dashboard";
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

function NavItem({
  item,
  active,
  collapsed,
  onClick,
}: {
  readonly item: NavItemDef;
  readonly active: boolean;
  readonly collapsed: boolean;
  readonly onClick?: () => void;
}) {
  return (
    <Link
      to={item.to}
      onClick={onClick}
      title={collapsed ? item.label : undefined}
      className={cn(
        "group/nav flex items-center rounded-lg py-2 text-[13px] overflow-hidden",
        "transition-[padding,gap,background-color,color] duration-300 ease-in-out",
        collapsed ? "justify-center px-0 gap-0" : "gap-3 px-3",
        active
          ? "bg-white/[0.06] font-medium text-foreground"
          : "text-muted-foreground hover:bg-white/[0.03] hover:text-foreground",
      )}
    >
      <item.icon
        className={cn(
          "h-[16px] w-[16px] shrink-0",
          active ? "text-nyx-secondary-400" : "text-text-tertiary",
        )}
      />
      <span
        className={cn(
          "truncate whitespace-nowrap transition-[opacity,max-width] duration-300 ease-in-out",
          collapsed ? "max-w-0 opacity-0" : "max-w-[160px] opacity-100",
        )}
      >
        {item.label}
      </span>
    </Link>
  );
}


function readMode(): SidebarMode {
  try {
    const v = localStorage.getItem(STORAGE_KEY);
    if (v === "expanded" || v === "collapsed" || v === "hover") return v;
  } catch {
    // ignore
  }
  return "expanded";
}

function writeMode(mode: SidebarMode) {
  try {
    localStorage.setItem(STORAGE_KEY, mode);
  } catch {
    // ignore
  }
}

export function Sidebar({
  onNavigate,
}: { readonly onNavigate?: () => void } = {}) {
  const routerState = useRouterState();
  const currentPath = routerState.location.pathname;

  const [mode, setMode] = useState<SidebarMode>(readMode);
  const [hovered, setHovered] = useState(false);
  const hoverTimerRef = useRef<ReturnType<typeof setTimeout> | null>(null);

  const changeMode = useCallback((next: SidebarMode) => {
    setMode(next);
    writeMode(next);
    setHovered(false);
  }, []);

  useEffect(() => {
    return () => {
      if (hoverTimerRef.current) clearTimeout(hoverTimerRef.current);
    };
  }, []);

  const isVisuallyExpanded =
    mode === "expanded" || (mode === "hover" && hovered);
  const isCollapsed = !isVisuallyExpanded;

  useEffect(() => {
    const width = mode === "expanded" ? "200px" : "52px";
    document.documentElement.style.setProperty("--sidebar-width", width);
  }, [mode]);

  function handleMouseEnter() {
    if (mode !== "hover") return;
    if (hoverTimerRef.current) clearTimeout(hoverTimerRef.current);
    hoverTimerRef.current = setTimeout(() => setHovered(true), 120);
  }

  function handleMouseLeave() {
    if (mode !== "hover") return;
    if (hoverTimerRef.current) clearTimeout(hoverTimerRef.current);
    hoverTimerRef.current = setTimeout(() => setHovered(false), 250);
  }

  const allItems = [...MAIN, ...APPROVALS, ...DEVELOPER];

  const sidebarContent = (
    <>
      <nav className="flex-1 overflow-y-auto scrollbar-none px-2 pt-2 pb-4">
        <div className="flex flex-col gap-[2px]">
          {MAIN.map((item) => (
            <NavItem
              key={item.to}
              item={item}
              active={isNavActive(item.to, currentPath, allItems)}
              collapsed={isCollapsed}
              onClick={onNavigate}
            />
          ))}
        </div>

        <div className="mx-2 my-2 border-t border-border/40" />
        <div className="flex flex-col gap-[2px]">
          {APPROVALS.map((item) => (
            <NavItem
              key={item.to}
              item={item}
              active={isNavActive(item.to, currentPath, allItems)}
              collapsed={isCollapsed}
              onClick={onNavigate}
            />
          ))}
        </div>

        <div className="mx-2 my-2 border-t border-border/40" />
        <div className="flex flex-col gap-[2px]">
          {DEVELOPER.map((item) => (
            <NavItem
              key={item.to}
              item={item}
              active={isNavActive(item.to, currentPath, allItems)}
              collapsed={isCollapsed}
              onClick={onNavigate}
            />
          ))}
        </div>
      </nav>

      {/* Sidebar control */}
      <div className="border-t border-border/60 px-2 py-2">
        <Popover>
          <PopoverTrigger asChild>
            <button
              className="flex h-[28px] w-[28px] items-center justify-center rounded-[6px] border border-white/[0.08] bg-white/[0.04] text-text-tertiary transition-colors duration-200 hover:border-white/[0.15] hover:text-foreground"
              title="Sidebar control"
            >
              {isCollapsed ? (
                <PanelLeft className="h-[14px] w-[14px]" />
              ) : (
                <PanelLeftClose className="h-[14px] w-[14px]" />
              )}
            </button>
          </PopoverTrigger>
          <PopoverContent
            side="top"
            align="start"
            className="w-[200px] rounded-xl border border-border/50 bg-card p-0 shadow-lg"
            sideOffset={8}
          >
            <div className="px-4 py-2.5 border-b border-border/50">
              <p className="text-[13px] font-medium text-foreground">
                Sidebar control
              </p>
            </div>
            <div className="p-1.5">
              <SidebarModeOption
                label="Expanded"
                active={mode === "expanded"}
                onClick={() => changeMode("expanded")}
              />
              <SidebarModeOption
                label="Collapsed"
                active={mode === "collapsed"}
                onClick={() => changeMode("collapsed")}
              />
              <SidebarModeOption
                label="Expand on hover"
                active={mode === "hover"}
                onClick={() => changeMode("hover")}
              />
            </div>
          </PopoverContent>
        </Popover>
      </div>
    </>
  );

  if (mode === "hover") {
    return (
      <aside
        className="relative h-full w-[52px] shrink-0"
        onMouseEnter={handleMouseEnter}
        onMouseLeave={handleMouseLeave}
      >
        <div
          className={cn(
            "absolute inset-y-0 left-0 z-30 flex flex-col border-r border-border/60 bg-background overflow-hidden",
            "transition-[width,box-shadow] duration-200 ease-out",
            hovered ? "w-[200px] shadow-xl shadow-black/20" : "w-[52px]",
          )}
        >
          {sidebarContent}
        </div>
      </aside>
    );
  }

  return (
    <aside
      className={cn(
        "flex h-full flex-col border-r border-border/60 overflow-hidden transition-[width] duration-300 ease-in-out",
        mode === "collapsed" ? "w-[52px]" : "w-[200px]",
      )}
    >
      {sidebarContent}
    </aside>
  );
}

function SidebarModeOption({
  label,
  active,
  onClick,
}: {
  readonly label: string;
  readonly active: boolean;
  readonly onClick: () => void;
}) {
  return (
    <button
      type="button"
      onClick={onClick}
      className={cn(
        "flex w-full items-center gap-2.5 rounded-lg px-3 py-2 text-[13px] transition-colors duration-200 hover:bg-white/[0.04]",
        active ? "text-foreground" : "text-muted-foreground hover:text-foreground",
      )}
    >
      <Circle
        className={cn(
          "h-2 w-2 shrink-0",
          active ? "fill-nyx-secondary-400 text-nyx-secondary-400" : "fill-transparent text-muted-foreground/30",
        )}
      />
      {label}
    </button>
  );
}
