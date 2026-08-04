import { useEffect, useCallback, useState, useMemo } from "react";
import { useNavigate } from "@tanstack/react-router";
import {
  LayoutDashboard,
  Cable,
  HardDrive,
  ShieldCheck,
  Settings,
  BookOpen,
  Sparkles,
  ClipboardList,
  Lock,
  Bell,
  Plus,
  KeyRound,
  Radio,
  Building2,
  Search,
  Code,
  BookMarked,
  Users,
  Ticket,
  Bot,
  Layers,
  Server,
  Plug,
} from "lucide-react";
import { cn } from "@/lib/utils";

type CommandGroup = "navigation" | "admin" | "action";

export interface CommandItem {
  readonly icon: typeof LayoutDashboard;
  readonly label: string;
  readonly group: CommandGroup;
  readonly to?: string;
  readonly search?: Record<string, string>;
  /**
   * Optional in-place action. Takes precedence over `to` when set, so an
   * entry can open a modal or fire a side-effect instead of navigating.
   */
  readonly onSelect?: () => void;
}

export const ALL_ITEMS: readonly CommandItem[] = [
  { icon: LayoutDashboard, label: "Dashboard", to: "/dashboard", group: "navigation" },
  { icon: Cable, label: "AI Services", to: "/keys", group: "navigation" },
  { icon: Building2, label: "Organizations", to: "/orgs", group: "navigation" },
  { icon: HardDrive, label: "Nodes", to: "/nodes", group: "navigation" },
  { icon: Radio, label: "Channel Bots", to: "/channel-bots", group: "navigation" },
  { icon: Bell, label: "Notifications", to: "/approvals/settings", group: "navigation" },
  { icon: ClipboardList, label: "Approval History", to: "/approvals/history", group: "navigation" },
  { icon: Lock, label: "Active Grants", to: "/approvals/grants", group: "navigation" },
  { icon: Code, label: "Developer Apps", to: "/developer/apps", group: "navigation" },
  { icon: Sparkles, label: "AI Setup", to: "/ai-setup", group: "navigation" },
  { icon: BookMarked, label: "Integration Guide", to: "/integration-guide", group: "navigation" },
  { icon: BookOpen, label: "Documentation", to: "/docs", group: "navigation" },
  { icon: Settings, label: "Account Settings", to: "/settings", group: "navigation" },
  { icon: KeyRound, label: "Authorized Apps", to: "/settings/consents", group: "navigation" },
  { icon: Users, label: "Admin Users", to: "/admin/users", group: "admin" },
  { icon: Ticket, label: "Invite Codes", to: "/admin/invite-codes", group: "admin" },
  { icon: ClipboardList, label: "Audit Log", to: "/admin/audit-log", group: "admin" },
  { icon: Bot, label: "Service Accounts", to: "/admin/service-accounts", group: "admin" },
  { icon: ShieldCheck, label: "Roles", to: "/admin/roles", group: "admin" },
  { icon: Layers, label: "Groups", to: "/admin/groups", group: "admin" },
  { icon: Server, label: "Services", to: "/services", group: "admin" },
  { icon: Plug, label: "Providers", to: "/providers", group: "admin" },
  {
    icon: Plus,
    label: "Connect a Service",
    to: "/keys",
    search: { tab: "services", action: "add-service" },
    group: "action",
  },
  {
    icon: KeyRound,
    label: "Create API key",
    to: "/keys",
    search: { tab: "nyxid", action: "create-key" },
    group: "action",
  },
  { icon: ShieldCheck, label: "Review approvals", to: "/approvals/history", group: "action" },
];

export function CommandPalette({
  open,
  onOpenChange,
}: {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
}) {
  const navigate = useNavigate();
  const [query, setQueryRaw] = useState("");
  const [selectedIndex, setSelectedIndex] = useState(0);

  const setQuery = useCallback((q: string) => {
    setQueryRaw(q);
    setSelectedIndex(0);
  }, []);

  const filtered = useMemo(() => {
    if (!query.trim()) return ALL_ITEMS.slice(0, 8);
    const q = query.toLowerCase();
    return ALL_ITEMS.filter(
      (item) =>
        item.label.toLowerCase().includes(q) ||
        (item.to?.toLowerCase().includes(q) ?? false),
    );
  }, [query]);

  const handleSelect = useCallback(
    (item: CommandItem) => {
      onOpenChange(false);
      setQuery("");
      if (item.onSelect) {
        item.onSelect();
        return;
      }
      if (item.to) {
        // TanStack `navigate` is typed against the route tree; cast through
        // `never` so this generic palette can target any whitelisted route
        // without each entry needing a literal route type.
        void navigate({
          to: item.to as never,
          search: (item.search ?? {}) as never,
        });
      }
    },
    [navigate, onOpenChange, setQuery],
  );

  useEffect(() => {
    if (!open) return;

    function handleKey(e: KeyboardEvent) {
      if (e.key === "Escape") {
        e.preventDefault();
        onOpenChange(false);
        setQuery("");
      }
      if (e.key === "ArrowDown") {
        e.preventDefault();
        setSelectedIndex((i) => Math.min(i + 1, filtered.length - 1));
      }
      if (e.key === "ArrowUp") {
        e.preventDefault();
        setSelectedIndex((i) => Math.max(i - 1, 0));
      }
      if (e.key === "Enter" && filtered.length > 0) {
        e.preventDefault();
        const item = filtered[selectedIndex];
        if (item) handleSelect(item);
      }
    }

    window.addEventListener("keydown", handleKey);
    return () => window.removeEventListener("keydown", handleKey);
  }, [open, filtered, selectedIndex, onOpenChange, handleSelect]);

  useEffect(() => {
    function handleGlobalKey(e: KeyboardEvent) {
      if (e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement) return;
      if (e.key === "k" && (e.metaKey || e.ctrlKey)) {
        e.preventDefault();
        onOpenChange(!open);
      }
      if (e.key === "/" && !open) {
        e.preventDefault();
        onOpenChange(true);
      }
    }
    window.addEventListener("keydown", handleGlobalKey);
    return () => window.removeEventListener("keydown", handleGlobalKey);
  }, [open, onOpenChange]);

  if (!open) return null;

  return (
    <div className="fixed inset-0 z-[100] flex items-start justify-center pt-[18vh]">
      <div
        className="fixed inset-0 bg-black/70 backdrop-blur-sm"
        onClick={() => { onOpenChange(false); setQuery(""); setSelectedIndex(0); }}
        role="button"
        tabIndex={-1}
        aria-label="Close search"
        onKeyDown={(e) => { if (e.key === "Escape") { onOpenChange(false); setQuery(""); } }}
      />

      <div className="relative z-10 w-full max-w-[640px] flex flex-col">
        {/* Search input bar */}
        <div className="flex items-center gap-3 rounded-2xl border border-white/[0.08] bg-[#1a1a1a] px-5 h-14">
          <Search className="h-5 w-5 shrink-0 text-text-tertiary" />
          <input
            type="text"
            value={query}
            onChange={(e) => setQuery(e.target.value)}
            placeholder="Search pages, actions, or keywords..."
            className="flex-1 bg-transparent text-[15px] text-foreground placeholder:text-text-tertiary outline-none"
            autoFocus
          />
          <kbd className="flex h-7 items-center rounded-lg border border-white/[0.08] bg-white/[0.04] px-2 text-[12px] font-medium text-text-tertiary">
            esc
          </kbd>
        </div>

        {/* Results */}
        {filtered.length > 0 && (
          <div className="mt-2 rounded-2xl border border-white/[0.08] bg-[#1a1a1a] p-2 max-h-[360px] overflow-y-auto">
            {filtered.map((item, i) => (
              <button
                key={`${item.to ?? "action"}-${item.label}`}
                type="button"
                onClick={() => handleSelect(item)}
                onMouseEnter={() => setSelectedIndex(i)}
                className={cn(
                  "flex w-full items-center gap-3 rounded-lg px-3 py-2.5 text-left text-[14px] transition-colors duration-300",
                  i === selectedIndex
                    ? "bg-white/[0.06] text-foreground"
                    : "text-muted-foreground hover:bg-white/[0.03]",
                )}
              >
                <item.icon
                  className={cn(
                    "h-4 w-4 shrink-0",
                    i === selectedIndex ? "text-nyx-secondary-400" : "text-text-tertiary",
                  )}
                />
                <span className="flex-1">{item.label}</span>
                {item.group === "action" && (
                  <span className="text-[11px] font-semibold uppercase tracking-[1.5px] text-text-tertiary">Action</span>
                )}
              </button>
            ))}
          </div>
        )}

        {query.trim() && filtered.length === 0 && (
          <div className="mt-2 rounded-2xl border border-white/[0.08] bg-[#1a1a1a] py-8 text-center text-[14px] text-text-tertiary">
            No results for &ldquo;{query}&rdquo;
          </div>
        )}
      </div>
    </div>
  );
}
