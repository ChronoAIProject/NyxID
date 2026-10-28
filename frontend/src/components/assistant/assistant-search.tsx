import { useState, type ComponentType, type ReactNode } from "react";
import { useQuery } from "@tanstack/react-query";
import { useNavigate, useRouterState } from "@tanstack/react-router";
import { Command } from "cmdk";
import {
  Bell,
  Bot,
  CalendarClock,
  ChartNoAxesCombined,
  House,
  KeyRound,
  LayoutGrid,
  MessageCircle,
  Monitor,
  Moon,
  Search,
  Settings,
  Settings2,
  ShieldCheck,
  SlidersHorizontal,
  Sun,
  SunMoon,
  Users,
  WalletCards,
} from "lucide-react";
import { Dialog, DialogContent, DialogDescription, DialogTitle } from "@/components/ui/dialog";
import { Tooltip, TooltipContent, TooltipProvider, TooltipTrigger } from "@/components/ui/tooltip";
import { SEARCH_SHORTCUT_ARIA, SEARCH_SHORTCUT_KEYS } from "@/hooks/use-assistant-search-shortcut";
import { useAssistantDrawerDismiss } from "@/components/assistant/assistant-drawer-context";
import { useAccountPanel } from "@/hooks/use-account-panel";
import { useFeature } from "@/hooks/use-feature-flag";
import { nyxBotQueryKeys, useNyxBotAgents } from "@/hooks/use-nyxbot-agents";
import { useNyxBotGroups } from "@/hooks/use-nyxbot-groups";
import { assistantJson } from "@/lib/assistant/assistant-http";
import type { AccountPanel, AccountPanelParams } from "@/lib/assistant/account-panel-search";
import { agentTitle } from "@/lib/assistant/nyxbot-labels";
import { parseAssistantSearch } from "@/lib/assistant/search";
import { FEATURE_FLAG } from "@/lib/feature-flags";
import { nyxAgentIndexSchema } from "@/schemas/assistant-nyxagent";
import { useAuthStore } from "@/stores/auth-store";
import { useThemeStore, type ThemeMode } from "@/stores/theme-store";
import { isBillingAvailable } from "@/types/api";

/** Recent chats shown before the user types; typing searches every loaded chat. */
const RECENT_PREVIEW = 8;
const RECENT_LIMIT = 50;

export function AssistantSearchButton({ onOpen }: { readonly onOpen: () => void }) {
  return (
    <TooltipProvider delayDuration={200}>
      <Tooltip>
        <TooltipTrigger asChild>
          <button
            type="button"
            onClick={onOpen}
            aria-label="Search"
            aria-keyshortcuts={SEARCH_SHORTCUT_ARIA}
            aria-haspopup="dialog"
            className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg text-text-tertiary transition-colors hover:bg-overlay hover:text-foreground focus-visible:outline-2 focus-visible:outline-ring"
          >
            <Search aria-hidden="true" className="h-4 w-4" />
          </button>
        </TooltipTrigger>
        <TooltipContent side="bottom" sideOffset={6} className="flex items-center gap-2 py-1 pr-1.5">
          Search
          <span aria-hidden="true" className="flex gap-0.5">
            {SEARCH_SHORTCUT_KEYS.map((key) => (
              <kbd key={key} className="flex h-5 min-w-5 items-center justify-center rounded-[4px] border border-hairline bg-overlay-strong px-1 font-sans text-10 text-text-tertiary">
                {key}
              </kbd>
            ))}
          </span>
        </TooltipContent>
      </Tooltip>
    </TooltipProvider>
  );
}

export function AssistantSearchDialog({
  open,
  onOpenChange,
}: {
  readonly open: boolean;
  readonly onOpenChange: (open: boolean) => void;
}) {
  return (
    <Dialog open={open} onOpenChange={onOpenChange}>
      <DialogContent
        scrollMode="body"
        className="md:max-w-2xl md:p-0 md:pt-0"
        aria-describedby={undefined}
      >
        <DialogTitle className="sr-only">Search</DialogTitle>
        <DialogDescription className="sr-only">Search chats, pages, settings and commands.</DialogDescription>
        {open ? <SearchCommand onDone={() => onOpenChange(false)} /> : null}
      </DialogContent>
    </Dialog>
  );
}

interface Entry {
  readonly id: string;
  readonly label: string;
  readonly icon: ComponentType<{ className?: string }>;
  readonly keywords?: readonly string[];
  readonly hint?: string;
  readonly run: () => void;
}

function SearchCommand({ onDone }: { readonly onDone: () => void }) {
  const [query, setQuery] = useState("");
  const typing = query.trim().length > 0;
  const navigate = useNavigate();
  const mock = useRouterState({
    select: (state) => parseAssistantSearch(state.location.search as Record<string, unknown>).mock,
  });
  const userId = useAuthStore((state) => state.user?.id);
  const billingAvailable = useAuthStore((state) => isBillingAvailable(state.user));
  const nyxagentEnabled = useFeature(FEATURE_FLAG.NYXAGENT_ENGINE);
  const agents = useNyxBotAgents(nyxagentEnabled);
  const groups = useNyxBotGroups(nyxagentEnabled);
  const recent = useQuery({
    queryKey: [...nyxBotQueryKeys.threads(userId), "search-recent"],
    queryFn: async () =>
      nyxAgentIndexSchema.parse(
        await assistantJson(`/assistant/nyxagent/conversations?limit=${RECENT_LIMIT}`),
      ).conversations,
    enabled: nyxagentEnabled && Boolean(userId),
  });
  const { prepareMenuOpen } = useAccountPanel();
  const dismissDrawer = useAssistantDrawerDismiss();
  const setTheme = useThemeStore((state) => state.setMode);

  function go(to: string, search: Record<string, unknown> = {}) {
    onDone();
    void navigate({ to: to as never, search: { ...search, ...(mock ? { mock: 1 } : {}) } as never });
  }
  function panel(name: AccountPanel, params: AccountPanelParams = {}) {
    // Let the dialog close (and hand focus back) before the panel opens.
    const openPanel = prepareMenuOpen(name, params, null, dismissDrawer);
    onDone();
    openPanel();
  }
  function theme(mode: ThemeMode) {
    setTheme(mode);
    onDone();
  }

  const agentList = (agents.data?.agents ?? []).filter((agent) => agent.status !== "destroyed");
  const agentNames = new Map(agentList.map((agent) => [agent.id, agentTitle(agent)]));
  const chats: Entry[] = (recent.data ?? [])
    .slice(0, typing ? RECENT_LIMIT : RECENT_PREVIEW)
    .map((chat) => ({
      id: `chat:${chat.id}`,
      label: chat.title || "New conversation",
      icon: MessageCircle,
      hint: chat.agent ? agentNames.get(chat.agent.id) : undefined,
      keywords: chat.agent ? [agentNames.get(chat.agent.id) ?? ""] : [],
      run: () => go("/assistant", { c: chat.id }),
    }));
  const people: Entry[] = typing
    ? [
        ...agentList.map((agent) => ({
          id: `agent:${agent.id}`,
          label: agentTitle(agent),
          icon: Bot,
          hint: agent.kind === "nyxbot" ? "NyxBot" : "Specialist",
          keywords: [agent.name, "agent"],
          run: () => go("/assistant", { agent: agent.id }),
        })),
        ...(groups.data ?? []).map((group) => ({
          id: `group:${group.id}`,
          label: group.name,
          icon: Users,
          hint: "Group",
          keywords: ["group", "crew"],
          run: () => go("/assistant", { g: group.id }),
        })),
      ]
    : [];
  const pages: Entry[] = [
    { id: "page:home", label: "Home", icon: House, run: () => go("/assistant") },
    { id: "page:plugins", label: "Plugins", icon: LayoutGrid, run: () => go("/assistant/plugins") },
    { id: "page:approvals", label: "Approvals", icon: ShieldCheck, run: () => go("/assistant/approvals") },
    { id: "page:automations", label: "Automations", icon: CalendarClock, keywords: ["schedule", "webhook", "trigger"], run: () => go("/assistant/automations") },
    { id: "page:machines", label: "Machines", icon: Monitor, keywords: ["node", "computer"], run: () => go("/assistant/machines") },
    { id: "page:logins", label: "Saved logins", icon: KeyRound, keywords: ["password", "machine"], run: () => go("/assistant/machines", { tab: "logins" }) },
  ];
  const settings: Entry[] = [
    ...([
      ["profile", "Profile", ["name", "email", "account"]],
      ["security", "Security", ["password", "mfa", "two-factor", "passkey"]],
      ["sessions", "Sessions", ["devices", "sign out"]],
      ["mcp", "MCP", ["claude", "codex", "tools"]],
      ["display", "Display", ["theme", "width", "text size", "sidebar", "motion"]],
      ["privacy", "Privacy", ["data", "telemetry", "delete"]],
    ] as const).map(([tab, label, keywords]) => ({
      id: `settings:${tab}`,
      label: `Settings: ${label}`,
      icon: Settings,
      keywords,
      run: () => panel("settings", { panelTab: tab }),
    })),
    ...(nyxagentEnabled
      ? [{ id: "nyxbot", label: "NyxBot settings", icon: Settings2, keywords: ["access mode", "channels", "bots"], run: () => panel("nyxbot") }]
      : []),
    ...(billingAvailable
      ? [
          { id: "billing", label: "Billing", icon: WalletCards, keywords: ["credits", "wallet", "top up"], run: () => panel("billing", { panelTab: "billing" }) },
          { id: "usage", label: "Usage", icon: ChartNoAxesCombined, keywords: ["cost", "tokens"], run: () => panel("billing", { panelTab: "usage" }) },
        ]
      : []),
    { id: "notifications", label: "Notification settings", icon: Bell, hint: "Studio", run: () => go("/approvals/settings") },
    { id: "studio", label: "Open Studio", icon: SlidersHorizontal, keywords: ["dashboard"], run: () => go("/dashboard") },
  ];
  const appearance: Entry[] = [
    { id: "theme:light", label: "Theme: Light", icon: Sun, keywords: ["appearance", "mode"], run: () => theme("light") },
    { id: "theme:dark", label: "Theme: Dark", icon: Moon, keywords: ["appearance", "mode"], run: () => theme("dark") },
    { id: "theme:system", label: "Theme: System", icon: SunMoon, keywords: ["appearance", "mode", "auto"], run: () => theme("system") },
  ];

  return (
    <Command label="Search" loop filter={matches} className="flex min-h-0 flex-1 flex-col">
      <div className="flex items-center gap-3 border-b border-border/60 px-5 pr-12 md:px-6 md:pr-12">
        <Search aria-hidden="true" className="h-4 w-4 shrink-0 text-text-tertiary" />
        <Command.Input
          value={query}
          onValueChange={setQuery}
          placeholder="Search chats, pages and settings..."
          className="h-14 w-full bg-transparent text-14 text-foreground outline-none placeholder:text-text-tertiary"
        />
      </div>
      <Command.List className="assistant-scrollbar min-h-0 flex-1 overflow-y-auto overscroll-contain p-2 md:max-h-[min(60vh,520px)]">
        <Command.Empty className="px-3 py-8 text-center text-13 text-text-tertiary">No results.</Command.Empty>
        {nyxagentEnabled ? (
          <Section heading="Recent chats">
            {recent.isPending ? (
              <Command.Loading>
                <p className="px-3 py-2 text-12 text-text-tertiary">Loading chats...</p>
              </Command.Loading>
            ) : null}
            {chats.map((entry) => <Row key={entry.id} entry={entry} />)}
          </Section>
        ) : null}
        {people.length ? (
          <Section heading="Agents and groups">
            {people.map((entry) => <Row key={entry.id} entry={entry} />)}
          </Section>
        ) : null}
        <Section heading="Go to">
          {pages.map((entry) => <Row key={entry.id} entry={entry} />)}
        </Section>
        {typing ? (
          <>
            <Section heading="Settings">
              {settings.map((entry) => <Row key={entry.id} entry={entry} />)}
            </Section>
            <Section heading="Appearance">
              {appearance.map((entry) => <Row key={entry.id} entry={entry} />)}
            </Section>
          </>
        ) : null}
      </Command.List>
    </Command>
  );
}

/**
 * Every typed word must appear in the entry's label, hint or keywords. Item
 * values are opaque ids, so they never match.
 */
function matches(_value: string, search: string, keywords?: string[]): number {
  const haystack = (keywords ?? []).join(" ").toLowerCase();
  const words = search.toLowerCase().split(/\s+/).filter(Boolean);
  return words.every((word) => haystack.includes(word)) ? 1 : 0;
}

function Section({ heading, children }: { readonly heading: string; readonly children: ReactNode }) {
  return (
    <Command.Group
      heading={heading}
      className="[&_[cmdk-group-heading]]:px-3 [&_[cmdk-group-heading]]:pb-1 [&_[cmdk-group-heading]]:pt-3 [&_[cmdk-group-heading]]:text-12 [&_[cmdk-group-heading]]:text-text-tertiary"
    >
      {children}
    </Command.Group>
  );
}

function Row({ entry }: { readonly entry: Entry }) {
  const Icon = entry.icon;
  return (
    <Command.Item
      value={entry.id}
      keywords={[entry.label, ...(entry.keywords ?? []), ...(entry.hint ? [entry.hint] : [])]}
      onSelect={entry.run}
      className="flex items-center gap-3 rounded-lg px-3 py-2 text-13 text-muted-foreground data-[selected=true]:bg-overlay-strong data-[selected=true]:text-foreground"
    >
      <Icon aria-hidden="true" className="h-4 w-4 shrink-0 text-text-tertiary" />
      <span className="min-w-0 flex-1 truncate">{entry.label}</span>
      {entry.hint ? <span className="shrink-0 text-11 text-text-tertiary">{entry.hint}</span> : null}
    </Command.Item>
  );
}
