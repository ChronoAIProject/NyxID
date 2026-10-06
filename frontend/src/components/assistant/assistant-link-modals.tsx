import { useOverlayLayer } from "@/lib/overlay-layer";
import { OverlayLayer } from "@/components/ui/overlay-layer";
import {
  useMemo,
  useRef,
  useState,
  type MouseEvent,
  type ReactNode,
} from "react";
import * as DialogPrimitive from "@radix-ui/react-dialog";
import { X } from "lucide-react";
import { ChannelBotSetup } from "@/components/channels/channel-bot-setup";
import { ConnectLinkContent } from "@/components/connect-link/connect-link-content";
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
}: {
  readonly target: Extract<
    ReturnType<typeof assistantModalLinkTarget>,
    { kind: "channel-bot" }
  >;
  readonly onClose: () => void;
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
    />
  );
}

function ConnectLinkModal({
  target,
  onClose,
}: {
  readonly target: Extract<
    ReturnType<typeof assistantModalLinkTarget>,
    { kind: "connect" }
  >;
  readonly onClose: () => void;
}) {
  const layer = useOverlayLayer();
  return (
    <Dialog open onOpenChange={(open) => !open && onClose()}>
      <DialogPortal>
        <DialogOverlay style={{ zIndex: layer - 1 }} />
        <OverlayLayer layer={layer}>
        <DialogPrimitive.Content
          aria-describedby={undefined}
          style={{ zIndex: layer }}
          className="fixed top-1/2 left-1/2 w-[calc(100%-2rem)] max-w-lg -translate-x-1/2 -translate-y-1/2 outline-none"
        >
          <DialogTitle className="sr-only">Connect a service</DialogTitle>
          <DialogBody className="max-h-[calc(100dvh-2rem)] rounded-xl bg-card shadow-xl shadow-primary/5">
            <ConnectLinkContent
              token={target.token}
              embedded
              redirectOnTerminal={false}
            />
          </DialogBody>
          <DialogClose className="absolute top-3 right-3 flex size-8 items-center justify-center rounded-full bg-card/80 text-muted-foreground transition-colors hover:bg-muted hover:text-foreground focus-visible:ring-2 focus-visible:ring-ring focus-visible:outline-none">
            <X className="size-4" aria-hidden="true" />
            <span className="sr-only">Close</span>
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
}: {
  readonly href: string | null;
  readonly onClose: () => void;
}) {
  const target = useMemo(
    () =>
      href && typeof window !== "undefined"
        ? assistantModalLinkTarget(href, window.location.origin)
        : null,
    [href],
  );
  if (!target) return null;
  if (target.kind === "connect") {
    return <ConnectLinkModal target={target} onClose={onClose} />;
  }
  return <ChannelBotSetupModal target={target} onClose={onClose} />;
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
  const [href, setHref] = useState<string | null>(null);
  const triggerRef = useRef<HTMLAnchorElement | null>(null);

  function closeModal() {
    setHref(null);
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
    event.preventDefault();
    triggerRef.current = link;
    setHref(targetLink.href);
  }

  return (
    <div className="contents" onClickCapture={handleClickCapture}>
      {children}
      <AssistantLinkModal href={href} onClose={closeModal} />
    </div>
  );
}
