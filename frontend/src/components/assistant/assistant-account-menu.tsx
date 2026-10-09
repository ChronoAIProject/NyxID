import { useAccountPanel } from "@/hooks/use-account-panel";
import type { AccountPanel, AccountPanelParams } from "@/lib/assistant/account-panel-search";
import { useRef, type MouseEvent, type ReactNode } from "react";
import { Link, useNavigate } from "@tanstack/react-router";
import {
  Bell,
  ChartNoAxesCombined,
  LogOut,
  Settings,
  Settings2,
  SlidersHorizontal,
  WalletCards,
} from "lucide-react";
import {
  DropdownMenu,
  DropdownMenuContent,
  DropdownMenuItem,
  DropdownMenuSeparator,
  DropdownMenuTrigger,
} from "@/components/ui/dropdown-menu";
import { useAssistantDrawerDismiss } from "@/components/assistant/assistant-drawer-context";
import { useLogout } from "@/hooks/use-auth";
import { useFeature } from "@/hooks/use-feature-flag";
import { FEATURE_FLAG } from "@/lib/feature-flags";
import { useAuthStore } from "@/stores/auth-store";
import { isBillingAvailable } from "@/types/api";

export function AssistantAccountMenuItems({ onPanel }: { readonly onPanel: (panel: AccountPanel, params?: AccountPanelParams) => void }) {
  const user = useAuthStore((state) => state.user);
  const billingAvailable = isBillingAvailable(user);
  const nyxagentEnabled = useFeature(FEATURE_FLAG.NYXAGENT_ENGINE);
  const dismissDrawer = useAssistantDrawerDismiss();
  const logout = useLogout();
  const navigate = useNavigate();

  function activateLink(event: MouseEvent<HTMLAnchorElement>) {
    if (
      !event.defaultPrevented &&
      event.button === 0 &&
      !event.metaKey &&
      !event.ctrlKey &&
      !event.shiftKey &&
      !event.altKey
    ) {
      dismissDrawer();
    }
  }

  async function handleLogout() {
    await logout.mutateAsync();
    dismissDrawer();
    void navigate({ to: "/login" });
  }

  return (
    <>
      <div className="px-3 py-1.5">
        <p className="truncate text-12 font-medium text-foreground">
          {user?.display_name ?? "User"}
        </p>
        <p className="truncate text-11 text-text-tertiary">
          {user?.email ?? ""}
        </p>
      </div>
      <DropdownMenuItem onSelect={() => onPanel("settings")}>
        <Settings aria-hidden="true" />Settings
      </DropdownMenuItem>
      {billingAvailable ? <>
        <DropdownMenuItem onSelect={() => onPanel("billing", { panelTab: "billing" })}>
          <WalletCards aria-hidden="true" />Billing
        </DropdownMenuItem>
        <DropdownMenuItem onSelect={() => onPanel("billing", { panelTab: "usage" })}>
          <ChartNoAxesCombined aria-hidden="true" />Usage
        </DropdownMenuItem>
      </> : null}
      <DropdownMenuItem asChild>
        <Link to="/approvals/settings" onClick={activateLink}>
          <Bell aria-hidden="true" />
          <span className="flex-1">Notification settings</span>
          <span aria-hidden="true" className="text-10 text-text-tertiary">
            Studio
          </span>
          <span className="sr-only"> (opens in Studio)</span>
        </Link>
      </DropdownMenuItem>
      {nyxagentEnabled ? (
        <DropdownMenuItem onSelect={() => onPanel("nyxbot")}>
          <Settings2 aria-hidden="true" />NyxBot settings
        </DropdownMenuItem>
      ) : null}
      <DropdownMenuSeparator />
      <DropdownMenuItem asChild>
        <Link to="/dashboard" onClick={activateLink}>
          <SlidersHorizontal aria-hidden="true" />
          Open Studio
        </Link>
      </DropdownMenuItem>
      <DropdownMenuSeparator />
      <DropdownMenuItem
        onSelect={() => void handleLogout()}
        className="text-destructive focus:text-destructive"
      >
        <LogOut aria-hidden="true" />
        Log out
      </DropdownMenuItem>
    </>
  );
}

export function AssistantAccountMenu({
  children,
  align = "end",
  side = "bottom",
}: {
  readonly children: ReactNode;
  readonly align?: "start" | "end";
  readonly side?: "top" | "bottom";
}) {
  const triggerRef = useRef<HTMLButtonElement>(null);
  const pending = useRef<(() => void) | null>(null);
  const { prepareMenuOpen } = useAccountPanel();
  const dismissDrawer = useAssistantDrawerDismiss();
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild ref={triggerRef}>{children}</DropdownMenuTrigger>
      <DropdownMenuContent align={align} side={side} className="w-56 p-2"
        onCloseAutoFocus={() => {
          const request = pending.current;
          pending.current = null;
          request?.();
        }}>
        <AssistantAccountMenuItems onPanel={(panel, params = {}) => { pending.current = prepareMenuOpen(panel, params, triggerRef.current, dismissDrawer); }} />
      </DropdownMenuContent>
    </DropdownMenu>
  );
}
