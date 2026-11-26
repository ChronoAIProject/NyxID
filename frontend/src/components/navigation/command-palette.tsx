import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { useEffect, useCallback, useState, useMemo, useId, useRef } from "react";
import * as Dialog from "@radix-ui/react-dialog";
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
  ChartNoAxesCombined,
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
  Webhook,
  CalendarClock,
} from "lucide-react";
import { cn } from "@/lib/utils";
import { canRestoreFocus } from "@/lib/assistant/panel-focus";
import { useAuthStore } from "@/stores/auth-store";
import { canAdminWrite } from "@/types/api";

type CommandGroup = "navigation" | "admin" | "action";

export interface CommandItem {
  readonly icon: typeof LayoutDashboard;
  readonly label: string;
  readonly group: CommandGroup;
  readonly to?: string;
  readonly search?: Record<string, string>;
  readonly requiresAdminWrite?: boolean;
  /**
   * Optional in-place action. Takes precedence over `to` when set, so an
   * entry can open a modal or fire a side-effect instead of navigating.
   */
  readonly onSelect?: () => void;
}

export const ALL_ITEMS: readonly CommandItem[] = [
  {icon: Bot, label: "Bot Setups", to: "/onboarding/setups", group: "navigation"},
  {icon: Bot, label: "Platform BotFather", to: "/admin/platform-bots", group: "admin", requiresAdminWrite: true},
  { icon: Bot, label: "Assistant", to: "/assistant", group: "navigation" },
  { icon: CalendarClock, label: "Automations", to: "/assistant/automations", group: "navigation" },
  { icon: Server, label: "Machines", to: "/assistant/machines", group: "navigation" },
  { icon: KeyRound, label: "Saved logins", to: "/assistant/machines", search: { tab: "logins" }, group: "navigation" },
  { icon: LayoutDashboard, label: "Dashboard", to: "/dashboard", group: "navigation" },
  { icon: Cable, label: "AI Services", to: "/keys", group: "navigation" },
  { icon: Building2, label: "Organizations", to: "/orgs", group: "navigation" },
  { icon: HardDrive, label: "Nodes", to: "/nodes", group: "navigation" },
  { icon: Radio, label: "Channel Bots", to: "/channel-bots", group: "navigation" },
  { icon: Bell, label: "Notifications", to: "/settings", search: { tab: "notifications" }, group: "navigation" },
  { icon: ClipboardList, label: "Approval History", to: "/approvals/history", group: "navigation" },
  { icon: Lock, label: "Active Grants", to: "/approvals/grants", group: "navigation" },
  { icon: Code, label: "Developer Apps", to: "/developer/apps", group: "navigation" },
  { icon: Webhook, label: "Triggers", to: "/triggers", group: "navigation" },
  { icon: Sparkles, label: "AI Setup", to: "/ai-setup", group: "navigation" },
  { icon: BookMarked, label: "Integration Guide", to: "/integration-guide", group: "navigation" },
  { icon: BookOpen, label: "Docs", to: "/docs", group: "navigation" },
  { icon: Settings, label: "Settings", to: "/settings", group: "navigation" },
  { icon: KeyRound, label: "Authorized Apps", to: "/settings/consents", group: "navigation" },
  { icon: Users, label: "Users", to: "/admin/users", group: "admin" },
  { icon: Ticket, label: "Invite Codes", to: "/admin/invite-codes", group: "admin" },
  { icon: ClipboardList, label: "Audit Log", to: "/admin/audit-log", group: "admin" },
  { icon: ChartNoAxesCombined, label: "Usage", to: "/admin/usage", group: "admin" },
  { icon: Bot, label: "Service Accounts", to: "/admin/service-accounts", group: "admin" },
  { icon: KeyRound, label: "OAuth Clients", to: "/admin/oauth-clients", group: "admin" },
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
  useUiTranslation();
  const navigate = useNavigate();
  const user = useAuthStore((state) => state.user);
  const listId = useId();
  const inputRef = useRef<HTMLInputElement>(null);
  const openerRef = useRef<HTMLElement | null>(null);
  const [query, setQueryRaw] = useState("");
  const [selectedIndex, setSelectedIndex] = useState(0);

  const setQuery = useCallback((q: string) => {
    setQueryRaw(q);
    setSelectedIndex(0);
  }, []);

  const handleOpenChange = useCallback((nextOpen: boolean) => {
    if (!nextOpen) setQuery("");
    onOpenChange(nextOpen);
  }, [onOpenChange, setQuery]);

  const filtered = useMemo(() => {
    const visibleItems = ALL_ITEMS.filter(
      (item) => !item.requiresAdminWrite || canAdminWrite(user),
    );
    if (!query.trim()) return visibleItems.slice(0, 8);
    const q = query.toLowerCase();
    return visibleItems.filter(
      (item) =>
        item.label.toLowerCase().includes(q) ||
        (item.to?.toLowerCase().includes(q) ?? false),
    );
  }, [query, user]);

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

  function handleInputKey(e: React.KeyboardEvent<HTMLInputElement>) {
    if (e.nativeEvent.isComposing) return;
    if (e.key === "ArrowDown" && filtered.length > 0) {
      e.preventDefault();
      setSelectedIndex((i) => Math.min(i + 1, filtered.length - 1));
    }
    if (e.key === "ArrowUp" && filtered.length > 0) {
      e.preventDefault();
      setSelectedIndex((i) => Math.max(i - 1, 0));
    }
    if (e.key === "Enter") {
      e.preventDefault();
      const item = filtered[Math.min(selectedIndex, filtered.length - 1)];
      if (item) handleSelect(item);
    }
  }

  useEffect(() => {
    function handleGlobalKey(e: KeyboardEvent) {
      if (open || e.defaultPrevented) return;
      if (e.target instanceof HTMLInputElement || e.target instanceof HTMLTextAreaElement) return;
      if (e.key === "k" && (e.metaKey || e.ctrlKey)) {
        e.preventDefault();
        onOpenChange(true);
      }
      if (e.key === "/") {
        e.preventDefault();
        onOpenChange(true);
      }
    }
    window.addEventListener("keydown", handleGlobalKey);
    return () => window.removeEventListener("keydown", handleGlobalKey);
  }, [open, onOpenChange]);

  const activeIndex = Math.min(selectedIndex, filtered.length - 1);
  const activeId = activeIndex >= 0 ? `${listId}-${activeIndex}` : undefined;
  useEffect(() => {
    if (open && activeId) {
      document.getElementById(activeId)?.scrollIntoView?.({ block: "nearest" });
    }
  }, [activeId, open]);

  return (
    <Dialog.Root open={open} onOpenChange={handleOpenChange}>
      <Dialog.Portal>
        <Dialog.Overlay
          data-command-overlay=""
          className="fixed inset-0 z-[100] bg-black/70 backdrop-blur-sm"
        />
        <Dialog.Content
          aria-describedby={undefined}
          className="fixed left-1/2 top-[18vh] z-[101] flex w-full max-w-[640px] -translate-x-1/2 flex-col outline-none"
          onKeyDown={(event) => event.stopPropagation()}
          onOpenAutoFocus={(event) => {
            event.preventDefault();
            openerRef.current = document.activeElement instanceof HTMLElement ? document.activeElement : null;
            inputRef.current?.focus();
          }}
          onCloseAutoFocus={(event) => {
            event.preventDefault();
            if (canRestoreFocus(openerRef.current)) openerRef.current.focus();
          }}
        >
          <Dialog.Title className="sr-only">{translateUi("Search pages, actions, or keywords...")}</Dialog.Title>
          {/* Search input bar */}
          <div className="flex items-center gap-3 rounded-2xl border border-hairline bg-popover shadow-lg px-5 h-14 focus-within:border-foreground focus-within:ring-2 focus-within:ring-foreground">
            <Search aria-hidden="true" className="h-5 w-5 shrink-0 text-muted-foreground" />
            <input
              ref={inputRef}
              type="text"
              role="combobox"
              aria-label={translateUi("Search pages, actions, or keywords...")}
              aria-autocomplete="list"
              aria-expanded={filtered.length > 0}
              aria-controls={filtered.length > 0 ? listId : undefined}
              aria-activedescendant={activeId}
              value={query}
              onChange={(e) => setQuery(e.target.value)}
              onKeyDown={handleInputKey}
              placeholder={translateUi("Search pages, actions, or keywords...")}
              className="flex-1 bg-transparent text-15 text-foreground placeholder:text-muted-foreground outline-none"
            />
            <Dialog.Close asChild>
              <button type="button" aria-label={translateUi("Close search")} className="flex h-7 items-center rounded-lg border border-hairline bg-overlay px-2 type-row-title text-muted-foreground focus-visible:outline-2 focus-visible:outline-offset-2 focus-visible:outline-foreground">
                <kbd>{translateUi("esc")}</kbd>
              </button>
            </Dialog.Close>
          </div>

          {/* Results */}
          {filtered.length > 0 && (
            <div id={listId} role="listbox" aria-label={translateUi("Search pages, actions, or keywords...")} className="mt-2 rounded-2xl border border-hairline bg-popover shadow-lg p-2 max-h-[360px] overflow-y-auto">
              {filtered.map((item, i) => (
                <button
                  key={`${item.to ?? "action"}-${item.label}`}
                  type="button"
                  role="option"
                  id={`${listId}-${i}`}
                  aria-selected={i === activeIndex}
                  tabIndex={-1}
                  onClick={() => handleSelect(item)}
                  onMouseEnter={() => setSelectedIndex(i)}
                  onMouseDown={(event) => event.preventDefault()}
                  className={cn(
                    "flex w-full items-center gap-3 rounded-lg border px-3 py-2.5 text-left text-14 transition-colors duration-300",
                    i === activeIndex
                      ? "border-foreground bg-overlay-strong text-foreground"
                      : "border-transparent text-muted-foreground hover:bg-overlay",
                  )}
                >
                  <item.icon aria-hidden="true" className="h-4 w-4 shrink-0 text-current" />
                  <span className="flex-1">{translateUi(item.label)}</span>
                  {item.group === "action" && (
                    <span className="type-secondary-strong uppercase tracking-[1.5px] text-muted-foreground">{translateUi("Action")}</span>
                  )}
                </button>
              ))}
            </div>
          )}

          {query.trim() && filtered.length === 0 && (
            <div role="status" className="mt-2 rounded-2xl border border-hairline bg-popover shadow-lg py-8 text-center text-14 text-muted-foreground">{translateUi("No results for “")}{query}&rdquo;
            </div>
          )}
        </Dialog.Content>
      </Dialog.Portal>
    </Dialog.Root>
  );
}
