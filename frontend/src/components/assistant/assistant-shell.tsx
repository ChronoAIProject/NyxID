import { AssistantAccountPanel } from "./assistant-account-panel";
import { useAssistantViewport } from "@/hooks/use-assistant-viewport";
import { ThreadTitle } from "@/components/assistant/thread-title";
import { useCallback, useEffect, useRef, useState, type CSSProperties, type ReactNode } from "react";
import { OverlayLayer } from "@/components/ui/overlay-layer";
import { ASSISTANT_OVERLAY_BASE } from "@/lib/overlay-layer";
import { Link, useRouter } from "@tanstack/react-router";
import { Menu, PanelLeftClose, PanelLeftOpen, X } from "lucide-react";
import { NyxidIcon } from "@/components/brand/nyxid-icon";
import { SidebarRailContext } from "./sidebar-rail-context";
import { useAssistantLayoutStore } from "@/stores/assistant-layout-store";
import { NyxidLogo } from "@/components/brand/nyxid-logo";
import { AssistantDrawerDismissContext } from "@/components/assistant/assistant-drawer-context";
import { useApplyTheme } from "@/hooks/use-theme";
import { useThemeStore } from "@/stores/theme-store";
import { SidebarResizeHandle } from "@/components/layout/sidebar-resize-handle";
import { useScrollFades } from "@/hooks/use-scroll-fades";
import { ChatContentLayout } from "./chat-content-layout";
import { OfflineBanner } from "./offline-banner";
import { ChatGripSlotContext } from "./chat-header-context";
import { AssistantSearchButton, AssistantSearchDialog } from "./assistant-search";
import { useAssistantSearchShortcut } from "@/hooks/use-assistant-search-shortcut";

export function AssistantShell({
  title,
  sidebar,
  headerActions,
  onRenameTitle,
  titleKey,
  resizableContent = false,
  aside,
  asidePercent,
  onAsidePercentChange,
  children,
}: {
  readonly title: string;
  readonly sidebar: ReactNode;
  readonly headerActions?: ReactNode;
  readonly onRenameTitle?: (title: string) => Promise<void>;
  readonly titleKey?: string;
  readonly resizableContent?: boolean;
  /** A column docked to the right of the content (the machine column); wide screens only. */
  readonly aside?: ReactNode;
  /** Share of the content area `aside` takes, in percent. */
  readonly asidePercent?: number;
  /** Set by dragging (or keying) the divider between the content and `aside`. */
  readonly onAsidePercentChange?: (percent: number) => void;
  readonly children: ReactNode;
}) {
  const [mobileSidebarOpen, setMobileSidebarOpen] = useState(false);
  const viewportRef = useAssistantViewport();
  useScrollFades(viewportRef);
  const savedSidebarWidth = useThemeStore((s) => s.sidebarWidths.assistant);
  const [previewWidth, setPreviewWidth] = useState<number | null>(null);
  const sidebarWidth = previewWidth ?? savedSidebarWidth;
  const dismissDrawer = useCallback(() => setMobileSidebarOpen(false), []);
  const [searchOpen, setSearchOpen] = useState(false);
  const [gripSlot, setGripSlot] = useState<HTMLDivElement | null>(null);
  const sidebarHidden = useAssistantLayoutStore((state) => state.sidebarHidden);
  const setSidebarHidden = useAssistantLayoutStore((state) => state.setSidebarHidden);
  const [asidePreview, setAsidePreview] = useState<number | null>(null);
  const shownAsidePercent = asidePreview ?? asidePercent ?? 50;
  const asideWidth = `${shownAsidePercent}%`;
  const openSearch = useCallback(() => setSearchOpen(true), []);
  useAssistantSearchShortcut(openSearch);
  const router = useRouter();
  useEffect(
    () =>
      router.subscribe("onResolved", ({ fromLocation, toLocation }) => {
        if (
          fromLocation?.pathname !== toLocation.pathname ||
          fromLocation?.searchStr !== toLocation.searchStr
        ) {
          dismissDrawer();
        }
      }),
    [router, dismissDrawer],
  );
  useApplyTheme();

  useEffect(() => {
    document.title = `nyxid - ${title}`;
  }, [title]);

  useEffect(() => {
    if (!mobileSidebarOpen) return;
    function closeOnEscape(event: KeyboardEvent) {
      // Radix layers opened inside the drawer (the row menu, its delete
      // confirmation) handle Escape on `document` in the capture phase and
      // preventDefault, well before this window-level listener. Escape must
      // dismiss the innermost layer only -- taking the drawer down with it
      // would unmount that layer's anchor and skip a whole step of the way
      // back in one keypress.
      if (event.defaultPrevented) return;
      if (event.key === "Escape") setMobileSidebarOpen(false);
    }
    window.addEventListener("keydown", closeOnEscape);
    return () => window.removeEventListener("keydown", closeOnEscape);
  }, [mobileSidebarOpen]);

  return (
    <AssistantDrawerDismissContext value={dismissDrawer}>
      <OverlayLayer layer={ASSISTANT_OVERLAY_BASE}>
        <div
          ref={viewportRef}
          // Desktop: the window is the sidebar's surface and the content
          // area sits on it as one rounded slab, with no divider lines.
          className="fixed inset-x-0 top-0 flex h-dvh flex-col overflow-hidden overscroll-none bg-background md:bg-sidebar"
          style={{
            paddingTop: "var(--sat)",
            paddingLeft: "var(--sal)",
            paddingRight: "var(--sar)",
          }}
        >
          <OfflineBanner />
          {/* Mobile only: the drawer opener. On desktop the sidebar column
              carries the brand and there is no top row. */}
          <header className="flex h-[52px] shrink-0 items-center gap-2 px-3 sm:px-4 md:hidden">
            <button
              type="button"
              onClick={() => setMobileSidebarOpen(true)}
              className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg border border-hairline text-text-tertiary"
              aria-label="Open chats"
              data-assistant-account-fallback="drawer"
            >
              <Menu className="h-4 w-4" />
            </button>
            <div className="min-w-0 flex-1 truncate text-12 text-muted-foreground">
              {onRenameTitle ? (
                <ThreadTitle key={titleKey} title={title} onRename={onRenameTitle} />
              ) : (
                title
              )}
            </div>
            <AssistantSearchButton onOpen={openSearch} />
          </header>

          <div className="flex min-h-0 flex-1 overflow-hidden">
            {sidebarHidden ? (
              // Hidden sidebar: a slim rail with home, the sidebar toggle and
              // search, then the sidebar itself drawn as icons and avatars.
              <aside className="hidden w-14 shrink-0 flex-col items-center bg-sidebar md:flex">
                <div className="flex shrink-0 flex-col items-center gap-1 pt-2.5 pb-1">
                  <Link
                    to="/assistant"
                    search={{}}
                    aria-label="Assistant home"
                    className="mb-1 flex h-8 w-8 items-center justify-center rounded-lg hover:bg-overlay"
                  >
                    <NyxidIcon className="h-5 w-5" />
                  </Link>
                  <SidebarToggle hidden onToggle={() => setSidebarHidden(false)} />
                  <AssistantSearchButton onOpen={openSearch} />
                  {headerActions}
                </div>
                <SidebarRailContext value>
                  <div className="min-h-0 w-full flex-1">{sidebar}</div>
                </SidebarRailContext>
              </aside>
            ) : (
              <aside
                className="relative hidden shrink-0 flex-col bg-sidebar md:flex"
                style={{ width: sidebarWidth }}
              >
                <div className="flex h-[52px] shrink-0 items-center gap-1 pl-4 pr-3">
                  <Link to="/assistant" search={{}} aria-label="Assistant home" className="mr-auto">
                    <NyxidLogo className="h-5 w-auto" />
                  </Link>
                  {headerActions}
                  <AssistantSearchButton onOpen={openSearch} />
                  <SidebarToggle hidden={false} onToggle={() => setSidebarHidden(true)} />
                </div>
                <div className="min-h-0 flex-1 overflow-hidden">{sidebar}</div>
                <SidebarResizeHandle
                  sidebar="assistant"
                  label="Resize sidebar"
                  onPreview={setPreviewWidth}
                />
              </aside>
            )}
            {/* The slab is glued to the window's right edge: no gap, and only
                its sidebar-facing corners are rounded. */}
            <div className="relative flex min-w-0 flex-1 md:mt-3 md:mb-2">
            <main className="flex min-w-0 flex-1 overflow-hidden bg-background md:rounded-l-2xl md:shadow-sm">
              <div className="h-full min-w-0 flex-1 overflow-hidden">
                {resizableContent ? (
                  <ChatGripSlotContext value={gripSlot}>
                    <ChatContentLayout>{children}</ChatContentLayout>
                  </ChatGripSlotContext>
                ) : children}
              </div>
              {aside ? (
                // Shares the content slab with the chat; no divider between them.
                <aside
                  aria-label="Machine"
                  className="hidden h-full shrink-0 lg:block"
                  style={{ width: asideWidth }}
                >
                  {aside}
                </aside>
              ) : null}
            </main>
            {aside && onAsidePercentChange ? (
              <AsideDivider
                percent={shownAsidePercent}
                onPreview={setAsidePreview}
                onChange={onAsidePercentChange}
              />
            ) : null}
            {/* Unclipped layer over the chat column for the chat width grips,
                whose tabs stand in the gap above the slab. */}
            <div
              ref={setGripSlot}
              className="pointer-events-none absolute inset-y-0 left-0 right-0 lg:right-[var(--assistant-aside-width,0px)]"
              style={{ "--assistant-aside-width": aside ? asideWidth : "0px" } as CSSProperties}
            />
            </div>
          </div>

          {mobileSidebarOpen && (
            <div className="fixed inset-0 z-[80] flex md:hidden">
              <button
                type="button"
                className="absolute inset-0 bg-background/80 backdrop-blur-sm"
                onClick={() => setMobileSidebarOpen(false)}
                aria-label="Close chats"
              />
              <aside
                className="relative flex h-full w-[min(320px,88vw)] flex-col border-r border-border bg-sidebar shadow-xl"
                style={{
                  paddingTop: "var(--sat)",
                  paddingBottom: "var(--sab)",
                  paddingLeft: "var(--sal)",
                }}
              >
                <div className="flex h-[52px] shrink-0 items-center gap-1 border-b border-border/60 px-4">
                  <NyxidLogo className="mr-auto h-5 w-auto" />
                  <AssistantSearchButton onOpen={openSearch} />
                  <button
                    type="button"
                    onClick={() => setMobileSidebarOpen(false)}
                    className="flex h-8 w-8 items-center justify-center rounded-lg text-text-tertiary"
                    aria-label="Close chats"
                  >
                    <X className="h-4 w-4" />
                  </button>
                </div>
                <div
                  className="min-h-0 flex-1"
                  onClick={(event) => {
                    const target = event.target as HTMLElement;
                    // React bubbles clicks from portaled layers (dialogs and menus
                    // opened from the drawer) through here; those are not drawer
                    // navigation, and closing would unmount them mid-action (a
                    // dialog's submit would never fire).
                    if (!event.currentTarget.contains(target)) return;
                    // Menu triggers open something anchored inside the drawer --
                    // closing it would unmount the anchor out from under them.
                    if (target.closest("[data-keep-drawer-open]")) return;
                    if (target.closest("a,button")) {
                      setMobileSidebarOpen(false);
                    }
                  }}
                >
                  {sidebar}
                </div>
              </aside>
            </div>
          )}
        </div>
        <AssistantSearchDialog open={searchOpen} onOpenChange={setSearchOpen} />
        <AssistantAccountPanel />
      </OverlayLayer>
    </AssistantDrawerDismissContext>
  );
}

/** Machine column bounds while dragging, mirrored from the store's clamp. */
const ASIDE_MIN = 25;
const ASIDE_MAX = 75;
const ASIDE_DEFAULT = 50;

/**
 * The divider between the chat and the docked column: drag (or arrow keys)
 * moves the split, double-click resets it. Its tab rises into the gap above
 * the slab like the chat width grips.
 */
function AsideDivider({
  percent,
  onPreview,
  onChange,
}: {
  readonly percent: number;
  readonly onPreview: (percent: number | null) => void;
  readonly onChange: (percent: number) => void;
}) {
  const drag = useRef<{ pointer: number; rect: DOMRect; current: number } | null>(null);
  const clamp = (value: number) => Math.round(Math.min(ASIDE_MAX, Math.max(ASIDE_MIN, value)));
  function cancel() {
    drag.current = null;
    onPreview(null);
  }
  return (
    <div
      role="separator"
      aria-label="Resize machine column"
      aria-orientation="vertical"
      aria-valuemin={ASIDE_MIN}
      aria-valuemax={ASIDE_MAX}
      aria-valuenow={percent}
      aria-valuetext={`Machine column ${percent}%`}
      tabIndex={0}
      title="Drag to resize · double-click to reset"
      className="group absolute -top-3 bottom-0 z-40 hidden w-5 translate-x-1/2 cursor-col-resize touch-none outline-none lg:block"
      style={{ right: `${percent}%` }}
      onPointerDown={(event) => {
        const wrapper = event.currentTarget.parentElement;
        if (event.button !== 0 || drag.current || !wrapper) return;
        event.preventDefault();
        event.currentTarget.setPointerCapture(event.pointerId);
        drag.current = { pointer: event.pointerId, rect: wrapper.getBoundingClientRect(), current: percent };
        onPreview(percent);
      }}
      onPointerMove={(event) => {
        const start = drag.current;
        if (!start || start.pointer !== event.pointerId) return;
        start.current = clamp(((start.rect.right - event.clientX) / start.rect.width) * 100);
        onPreview(start.current);
      }}
      onPointerUp={(event) => {
        if (drag.current?.pointer !== event.pointerId) return;
        onChange(drag.current.current);
        cancel();
      }}
      onPointerCancel={cancel}
      onLostPointerCapture={cancel}
      onDoubleClick={() => onChange(ASIDE_DEFAULT)}
      onKeyDown={(event) => {
        if (event.key === "Escape") { cancel(); return; }
        // The divider moves with the arrow; the machine column grows leftwards.
        const delta = { ArrowLeft: 2, ArrowRight: -2 }[event.key];
        if (delta !== undefined) {
          event.preventDefault();
          onChange(clamp(percent + delta));
        } else if (event.key === "Home" || event.key === "End") {
          event.preventDefault();
          onChange(event.key === "Home" ? ASIDE_MAX : ASIDE_MIN);
        }
      }}
    >
      <span
        aria-hidden="true"
        data-aside-divider-grip=""
        className="pointer-events-none absolute top-1 left-1/2 flex h-3 w-5 -translate-x-1/2 items-center justify-center rounded-t-[5px] bg-background text-text-tertiary transition-colors group-hover:text-foreground group-focus-visible:text-foreground motion-reduce:transition-none"
      >
        <svg viewBox="0 0 12 8" className="h-2 w-3 fill-current">
          <path d="M0 4 4 1v6Zm12 0L8 1v6ZM5.5 0h1v8h-1Z" />
        </svg>
      </span>
      <span
        aria-hidden="true"
        className="pointer-events-none absolute top-3 bottom-0 left-1/2 w-px -translate-x-1/2 bg-input-focus opacity-0 transition-opacity duration-150 group-hover:opacity-100 group-focus-visible:opacity-100 group-active:opacity-100 motion-reduce:transition-none"
      />
    </div>
  );
}

function SidebarToggle({ hidden, onToggle }: { readonly hidden: boolean; readonly onToggle: () => void }) {
  const label = hidden ? "Show sidebar" : "Hide sidebar";
  return (
    <button
      type="button"
      onClick={onToggle}
      aria-label={label}
      title={label}
      className="flex h-8 w-8 shrink-0 items-center justify-center rounded-lg text-text-tertiary transition-colors hover:bg-overlay hover:text-foreground focus-visible:outline-2 focus-visible:outline-ring"
    >
      {hidden ? <PanelLeftOpen aria-hidden="true" className="h-4 w-4" /> : <PanelLeftClose aria-hidden="true" className="h-4 w-4" />}
    </button>
  );
}
