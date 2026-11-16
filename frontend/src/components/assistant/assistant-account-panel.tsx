import { accountSettingsSections, ACCOUNT_SETTINGS_TITLES } from "@/lib/account-settings-sections";
import { billingSections, BILLING_SECTION_TITLES } from "@/lib/billing-sections";
import { parseNyxBotSettingsSection, NYXBOT_SETTINGS_TITLES } from "@/lib/assistant/nyxbot-settings-sections";
import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { CompactChrome } from "@/components/ui/compact-title";
import {
  GroupedDialog,
  GroupedDialogContent,
} from "@/components/ui/grouped-dialog";
import { DialogFocusReturnContext } from "@/components/ui/dialog-focus-return";
import { lazy, Suspense, useEffect, useState } from "react";
import {
  DialogDescription, DialogTitle,
} from "@/components/ui/dialog";
import { useAccountPanel } from "@/hooks/use-account-panel";
import { accountPanelAllowed } from "@/lib/assistant/account-panel-availability";
import { restorePanelFocus } from "@/lib/assistant/panel-focus";
import { useAuthStore } from "@/stores/auth-store";
import { useCreditsDenialStore } from "@/stores/credits-denial-store";
import type { AccountPanel } from "@/lib/assistant/account-panel-search";

const Settings = lazy(() => import("@/pages/settings").then((m) => ({ default: m.SettingsPage })));
const Billing = lazy(() => import("@/pages/billing").then((m) => ({ default: m.BillingPage })));
const NyxBot = lazy(() => import("./nyxbot-settings-content").then((m) => ({ default: m.NyxBotSettingsContent })));
const Notifications = lazy(() => import("./notifications-panel").then((m) => ({ default: m.NotificationsPanel })));
const panels: Record<Exclude<AccountPanel, "notifications">, { title: string; description: string; width: string }> = {
  settings: { title: "Account Settings", description: "Manage your account settings and preferences.", width: "md:max-w-3xl" },
  billing: { title: "Billing & Usage", description: "Your balance, benefits, and usage in one place.", width: "md:max-w-5xl" },
  nyxbot: { title: "NyxBot settings", description: "Manage permissions, tasks, automations, notifications, and chats for your agents.", width: "md:max-w-2xl" },
};

export function AssistantAccountPanel() {
  useUiTranslation();
  const { current, close, opener } = useAccountPanel();
  const user = useAuthStore((s) => s.user);
  const loading = useAuthStore((s) => s.isLoading);
  const authenticated = useAuthStore((s) => s.isAuthenticated);
  const denial = useCreditsDenialStore((s) => s.current);
  const ready = !loading && authenticated && user !== null;
  const allowed = ready && current.panel && accountPanelAllowed(current.panel, user);
  const [admitted, setAdmitted] = useState<{ panel: AccountPanel; actor: string } | null>(null);
  const alreadyMounted = admitted?.panel === current.panel && admitted?.actor === user?.id;
  // Keep an existing registration beneath credits during refresh. New registrations
  // wait for credits teardown, so Radix's last registered layer is also visually top.
  const retainUnderCredits = alreadyMounted && Boolean(denial) && loading && authenticated && user !== null;
  const render = Boolean(current.panel && (allowed || retainUnderCredits) && (!denial || alreadyMounted));
  if (render && current.panel && user && !alreadyMounted) setAdmitted({ panel: current.panel, actor: user.id });
  else if (!render && admitted !== null) setAdmitted(null);
  useEffect(() => {
    if (ready && current.panel && !allowed) void close("replace");
  }, [ready, current.panel, allowed, close]);
  if (!render || !current.panel) return null;
  if (current.panel === "notifications") {
    // The feed is a sheet beside the chat, not a settings dialog.
    return (
      <Suspense fallback={null}>
        <Notifications onClose={() => void close()} opener={opener} />
      </Suspense>
    );
  }
  const panel = panels[current.panel];
  const section = current.panel === "nyxbot" ? parseNyxBotSettingsSection(current.panelTab) : undefined;
  const accountSection = current.panel === "settings" ? accountSettingsSections.parse(current.panelTab) : undefined;
  const billingSection = current.panel === "billing" ? billingSections.parse(current.panelSection) : undefined;
  const dialogTitle = section ? NYXBOT_SETTINGS_TITLES[section] : accountSection ? ACCOUNT_SETTINGS_TITLES[accountSection] : billingSection ? BILLING_SECTION_TITLES[billingSection] : panel.title;
  return (
    <GroupedDialog
      open
      onOpenChange={(open) => { if (!open) void close(); }}
    >
      <GroupedDialogContent
        scrollMode="body"
        className={`[&>button]:z-40 ${panel.width} md:h-[min(48rem,85dvh)]`}
        onCloseAutoFocus={(event) => {
          event.preventDefault();
          restorePanelFocus(opener());
        }}
      >
        <DialogTitle asChild><span className="sr-only">{translateUi(dialogTitle)}</span></DialogTitle>
        <DialogDescription className="sr-only">{translateUi(panel.description)}</DialogDescription>
        {current.panel !== "nyxbot" && !accountSection && !billingSection && <CompactChrome className="shrink-0 pr-6"><h2 className="type-card-title">{translateUi(panel.title)}</h2></CompactChrome>}
        <div className="flex min-h-0 flex-1 flex-col" style={{ paddingBottom: "max(.25rem, var(--sab))" }}>
          <DialogFocusReturnContext value={true}>
          <Suspense fallback={<p role="status" className="text-14 text-text-tertiary">{translateUi("Loading ")}{translateUi(panel.title)}...</p>}>
            {current.panel === "settings" ? <Settings presentation="panel" /> :
              current.panel === "billing" ? <Billing presentation="panel" /> : <NyxBot />}
          </Suspense>
          </DialogFocusReturnContext>
        </div>
      </GroupedDialogContent>
    </GroupedDialog>
  );
}
