import { useUiTranslation } from "@/i18n/use-ui-translation";
import { useAccountPanel } from "@/hooks/use-account-panel";
import { useOverlayLayer } from "@/lib/overlay-layer";
import { OverlayLayer } from "@/components/ui/overlay-layer";
import {
  useMemo,
  useContext,
  useRef,
  useState,
  useSyncExternalStore,
  type MouseEvent,
  type ReactNode,
} from "react";
import * as DialogPrimitive from "@radix-ui/react-dialog";
import { X } from "lucide-react";
import { ChannelBotSetup } from "@/components/channels/channel-bot-setup";
import { ConnectLinkContent } from "@/components/connect-link/connect-link-content";
import { ChannelConnectLinkContent } from "@/components/channels/channel-connect-link-content";
import { ApiKeyCreateDialog } from "@/components/dashboard/api-key-create-dialog";
import { NyxBotHandoffContext } from "@/lib/assistant/nyxbot-handoff-context";
import type { PlatformSetupCompletion } from "@/lib/assistant/platform-setup";
import { getAssistantIdentityRevision, subscribeAssistantIdentity } from "@/lib/assistant/identity";
import {
  Dialog,
  DialogBody,
  DialogClose,
  DialogOverlay,
  DialogPortal,
  DialogTitle,
} from "@/components/ui/dialog";
import { assistantModalLinkTarget } from "@/lib/assistant/assistant-link-target";
import { channelBotSetupUrlValues } from "@/schemas/channel-bot-setup";
import type { ChannelPlatform } from "@/types/channels";

function ChannelBotSetupModal({
  target,
  onClose,
  onComplete,
}: {
  readonly target: Extract<
    ReturnType<typeof assistantModalLinkTarget>,
    { kind: "channel-bot" }
  >;
  readonly onClose: () => void;
  readonly onComplete?: (result: PlatformSetupCompletion) => Promise<void>;
}) {
  const values = useMemo(
    () => channelBotSetupUrlValues(target.search),
    [target.search],
  );
  const defaultLabel = values.label?.trim().slice(0, 128) ?? "";

  // ChannelBotSetup already owns the Radix dialog for its reusable dialog
  // presentation. Reusing it directly avoids nested overlays while keeping
  // the full-page route's existing caller and contract unchanged.
  return (
    <ChannelBotSetup
      open
      onOpenChange={(open) => {
        if (!open) onClose();
      }}
      defaultPlatform={target.platform as ChannelPlatform}
      defaultLabel={defaultLabel}
      defaultOrgId={values.target_org_id ?? null}
      stayInPlace
      initialTelegramRequestId={target.requestId}
      onFinish={onComplete && !target.requestId ? (id) => onComplete({ kind: "channel_bot", id }) : undefined}
    />
  );
}

function ConnectLinkModal({
  target,
  onClose,
}: {
  readonly target: Extract<
    ReturnType<typeof assistantModalLinkTarget>,
    { kind: "connect" | "channel-connect" }
  >;
  readonly onClose: () => void;
}) {
  const { t } = useUiTranslation();
  const layer = useOverlayLayer();
  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogPortal>
        <DialogOverlay style={{ zIndex: layer - 1 }} />
        <OverlayLayer layer={layer}>
        <DialogPrimitive.Content
          onInteractOutside={(event) => { if (target.kind === "channel-connect") event.preventDefault(); }}
          onEscapeKeyDown={(event) => { if (target.kind === "channel-connect") event.preventDefault(); }}
          aria-describedby={undefined}
          style={{ zIndex: layer }}
          className="fixed top-1/2 left-1/2 w-[calc(100%-2rem)] max-w-lg -translate-x-1/2 -translate-y-1/2 outline-none"
        >
          <DialogTitle className="sr-only">{target.kind === "channel-connect" ? t("Set up your channel bot") : t("Connect a service")}</DialogTitle>
          <DialogBody className="max-h-[calc(100dvh-2rem)] rounded-xl bg-card shadow-xl shadow-primary/5">
            {target.kind === "channel-connect" ? <ChannelConnectLinkContent token={target.token} embedded onClose={onClose} /> : <ConnectLinkContent
              token={target.token}
              embedded
              redirectOnTerminal={false}
            />}
          </DialogBody>
          <DialogClose className="absolute top-3 right-3 flex size-8 items-center justify-center rounded-full bg-card/80 text-muted-foreground transition-colors hover:bg-muted hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none">
            <X className="size-4" aria-hidden="true" />
            <span className="sr-only">{t("Close")}</span>
          </DialogClose>
        </DialogPrimitive.Content>
        </OverlayLayer>
      </DialogPortal>
    </Dialog>
  );
}

export function AssistantLinkModal({
  href,
  onClose,
  onComplete,
}: {
  readonly href: string | null;
  readonly onClose: () => void;
  readonly onComplete?: (result: PlatformSetupCompletion) => Promise<void>;
}) {
  const target = useMemo(
    () =>
      href && typeof window !== "undefined"
        ? assistantModalLinkTarget(href, window.location.origin)
        : null,
    [href],
  );
  if (!target) return null;
  if (target.kind === "connect" || target.kind === "channel-connect") {
    return <ConnectLinkModal target={target} onClose={onClose} />;
  }
  if (target.kind === "agent-key") {
    return <ApiKeyCreateDialog externalOpen onExternalOpenChange={(open) => !open && onClose()} hideTrigger onComplete={onComplete ? (id) => onComplete({ kind: "agent_key", id }) : undefined} />;
  }
  if (target.kind === "account-panel" || target.kind === "conversation" || target.kind === "workspace") return null;
  return <ChannelBotSetupModal target={target} onClose={onClose} onComplete={onComplete} />;
}

/**
 * Page-level adapter for the shared markdown/link components. The link
 * components stay intentionally unaware of Nyxbot-specific modal behavior.
 */
export function AssistantLinkModalHost({
  children,
}: {
  readonly children: ReactNode;
}) {
  const account = useAccountPanel();
  const handoff = useContext(NyxBotHandoffContext);
  const identity = useSyncExternalStore(subscribeAssistantIdentity, getAssistantIdentityRevision, getAssistantIdentityRevision);
  const [selection, setSelection] = useState<{ href: string; identity: number; complete?: (result: PlatformSetupCompletion) => Promise<void> } | null>(null);
  const href = selection?.identity === identity ? selection.href : null;
  const triggerRef = useRef<HTMLAnchorElement | null>(null);

  function closeModal() {
    setSelection(null);
    const trigger = triggerRef.current;
    requestAnimationFrame(() => {
      if (trigger?.isConnected) trigger.focus();
    });
  }
  function handleClickCapture(event: MouseEvent<HTMLDivElement>) {
    if (
      event.defaultPrevented ||
      event.button !== 0 ||
      event.metaKey ||
      event.ctrlKey ||
      event.shiftKey ||
      event.altKey
    ) {
      return;
    }
    const target = event.target;
    if (!(target instanceof Element)) return;
    const link = target.closest("a");
    if (!link || !event.currentTarget.contains(link)) return;
    const targetLink = assistantModalLinkTarget(
      link.href,
      window.location.origin,
    );
    if (!targetLink) return;
    if (targetLink.kind === "workspace") {
      if (handoff?.navigate) { event.preventDefault(); handoff.navigate(targetLink.href); }
      return;
    }
    if (targetLink.kind === "conversation") {
      if (handoff) {
        event.preventDefault();
        handoff.open(targetLink.conversationId);
      }
      return;
    }
    event.preventDefault();
    if (targetLink.kind === "account-panel") {
      void account.open(targetLink.panel, targetLink.params, link);
      return;
    }
    triggerRef.current = link;
    setSelection({ href: targetLink.href, identity, complete: handoff?.complete });
  }

  return (
    <div className="contents" onClickCapture={handleClickCapture}>
      {children}
      <AssistantLinkModal key={identity} href={href} onClose={closeModal} onComplete={selection?.identity === identity ? selection.complete : undefined} />
    </div>
  );
}
