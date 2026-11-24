import { useUiTranslation } from "@/i18n/use-ui-translation";
import { translateUi } from "@/i18n/use-ui-translation";
import { useAccountPanel } from "@/hooks/use-account-panel";
import type { AccountPanel, AccountPanelParams } from "@/lib/assistant/account-panel-search";
import { useRef, type MouseEvent, type ReactNode } from "react";
import { Link, useNavigate } from "@tanstack/react-router";
import {
  ChartNoAxesCombined,
  LogOut,
  Settings,
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
import { useAuthStore } from "@/stores/auth-store";
import { isBillingAvailable } from "@/types/api";

export function AssistantAccountMenuItems({ onPanel }: { readonly onPanel: (panel: AccountPanel, params?: AccountPanelParams) => void }) {
  useUiTranslation();
  const user = useAuthStore((state) => state.user);
  const billingAvailable = isBillingAvailable(user);
  const dismissDrawer = useAssistantDrawerDismiss();
  const logout = useLogout();
  const navigate = useNavigate();

  async function handleLogout() {
    await logout.mutateAsync();
    dismissDrawer();
    void navigate({ to: "/login" });
  }

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

  return (
    <>
      {user?.email && (
        <div className="shell-account-heading">
          <p className="truncate shell-user-email" title={user.email}>{user.email}</p>
        </div>
      )}
      <DropdownMenuItem onSelect={() => onPanel("settings")}>
        <Settings aria-hidden="true" />{translateUi("Settings")}</DropdownMenuItem>
      {billingAvailable ? <>
        <DropdownMenuItem onSelect={() => onPanel("billing", { panelTab: "billing" })}>
          <WalletCards aria-hidden="true" />{translateUi("Billing")}</DropdownMenuItem>
        <DropdownMenuItem onSelect={() => onPanel("billing", { panelTab: "usage" })}>
          <ChartNoAxesCombined aria-hidden="true" />{translateUi("Usage")}</DropdownMenuItem>
      </> : null}
      <DropdownMenuSeparator />
      <DropdownMenuItem asChild>
        <Link to="/dashboard" onClick={activateLink}>
          <SlidersHorizontal aria-hidden="true" />{translateUi("Open Studio")}</Link>
      </DropdownMenuItem>
      <DropdownMenuSeparator />
      <DropdownMenuItem onSelect={() => void handleLogout()} disabled={logout.isPending}>
        <LogOut aria-hidden="true" />{translateUi("Log out")}
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
  useUiTranslation();
  const triggerRef = useRef<HTMLButtonElement>(null);
  const pending = useRef<(() => void) | null>(null);
  const { prepareMenuOpen } = useAccountPanel();
  const dismissDrawer = useAssistantDrawerDismiss();
  return (
    <DropdownMenu>
      <DropdownMenuTrigger asChild ref={triggerRef}>{children}</DropdownMenuTrigger>
      <DropdownMenuContent align={align} side={side} className="w-[max(184px,var(--radix-dropdown-menu-trigger-width))] min-w-0 max-w-[calc(100vw-1rem)] max-h-[var(--radix-dropdown-menu-content-available-height)] overflow-y-auto p-2"
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
