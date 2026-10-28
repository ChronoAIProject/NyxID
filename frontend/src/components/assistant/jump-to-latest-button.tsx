import { ArrowDown } from "lucide-react";
import { AnimatePresence, MotionConfig, motion } from "motion/react";

/**
 * Takes a reader who scrolled up back to the newest message. Centered on the
 * chat column just above the composer, inside the transcript's positioned
 * parent.
 */
export function JumpToLatestButton({
  visible,
  bottomInset,
  onJump,
}: {
  readonly visible: boolean;
  /** Composer height, used until the layout reports `--assistant-composer-inset`. */
  readonly bottomInset: number;
  readonly onJump: () => void;
}) {
  return (
    <div
      className="pointer-events-none absolute left-0 z-20 px-4 sm:px-6"
      style={{
        bottom: `calc(var(--assistant-composer-inset, ${String(bottomInset)}px) + 12px)`,
        width: "calc(100% - var(--assistant-content-inset, 0px))",
      }}
    >
      <div className="mx-auto flex w-full max-w-[var(--chat-content-width,758px)] justify-center">
        <MotionConfig reducedMotion="user">
          <AnimatePresence initial={false}>
            {visible ? (
              <motion.button
                key="jump-to-latest"
                type="button"
                aria-label="Jump to latest message"
                title="Jump to latest message"
                initial={{ opacity: 0, y: 8 }}
                animate={{ opacity: 1, y: 0 }}
                exit={{ opacity: 0, y: 8 }}
                transition={{ duration: 0.15, ease: "easeOut" }}
                onClick={onJump}
                className="pointer-events-auto flex size-8 items-center justify-center rounded-full border border-hairline bg-background/95 text-muted-foreground shadow-sm backdrop-blur-sm transition-colors hover:border-hairline-strong hover:text-foreground focus-visible:border-input-focus focus-visible:outline-none"
              >
                <ArrowDown aria-hidden="true" className="size-4" />
              </motion.button>
            ) : null}
          </AnimatePresence>
        </MotionConfig>
      </div>
    </div>
  );
}
