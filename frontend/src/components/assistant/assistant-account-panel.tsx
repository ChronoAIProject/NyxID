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
  Dialog, DialogBody, DialogContent, DialogDescription, DialogHeader, DialogTitle,
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
const panels = {
  settings: { title: "Account Settings", description: "Manage your account settings and preferences.", width: "md:max-w-3xl" },
  billing: { title: "Billing & Usage", description: "Your balance, benefits, and usage in one place.", width: "md:max-w-5xl" },
  nyxbot: { title: "NyxBot settings", description: "NyxBot runs with full access to your connected services and account; specialists use only what you or NyxBot grant them. These settings apply to all of your agents.", width: "md:max-w-2xl" },
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
  const panel = panels[current.panel];
  const grouped = current.panel !== "billing";
  const PanelDialog = grouped ? GroupedDialog : Dialog;
  const PanelBody = grouped ? "div" : DialogBody;
  const PanelContent =
    grouped ? GroupedDialogContent : DialogContent;
  return (
    <PanelDialog
      open
      onOpenChange={(open) => { if (!open) void close(); }}
    >
      <PanelContent
        scrollMode="body"
        className={`${panel.width}${current.panel === "settings" ? " md:h-[min(48rem,85dvh)]" : ""}`}
        onCloseAutoFocus={(event) => {
          event.preventDefault();
          restorePanelFocus(opener());
        }}
      >
        <CompactChrome className="[[data-compact=true]]:border-b [[data-compact=true]]:border-border/50"><DialogHeader className="shrink-0 pr-6">
          <DialogTitle className="[[data-compact=true]_&]:text-15">{translateUi(panel.title)}</DialogTitle>
          <DialogDescription className="[[data-compact=true]_&]:hidden">{translateUi(panel.description)}</DialogDescription>
        </DialogHeader></CompactChrome>
        <PanelBody className={grouped ? "flex min-h-0 flex-1 flex-col" : undefined} style={{ paddingBottom: "max(.25rem, var(--sab))" }}>
          <DialogFocusReturnContext value={true}>
          <Suspense fallback={<p role="status" className="text-14 text-text-tertiary">{translateUi("Loading ")}{translateUi(panel.title)}...</p>}>
            {current.panel === "settings" ? <Settings presentation="panel" /> :
              current.panel === "billing" ? <Billing presentation="panel" /> : <NyxBot />}
          </Suspense>
          </DialogFocusReturnContext>
        </PanelBody>
      </PanelContent>
    </PanelDialog>
  );
}
