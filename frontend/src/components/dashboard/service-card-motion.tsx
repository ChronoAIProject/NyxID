import {
  createContext,
  useContext,
  useRef,
  type ComponentProps,
  type ReactNode,
} from "react";
import { AnimatePresence, motion } from "motion/react";
import {
  REFLOW,
  REVEAL_DELAY,
  useRevealMotion,
} from "@/hooks/use-card-sequence";
import { cn } from "@/lib/utils";

const LayoutKey = createContext<unknown>(undefined);

function useLayoutProps() {
  const layoutDependency = useContext(LayoutKey);
  return { layoutDependency, transition: { layout: REFLOW } } as const;
}

/**
 * A card that animates between grid positions. `layoutKey` must change
 * exactly when card spans change, so unrelated renders never animate.
 */
export function MotionCard({
  layoutKey,
  className,
  style,
  ref,
  ...props
}: ComponentProps<typeof motion.section> & { readonly layoutKey: unknown }) {
  // Clip only while moving: glided children render at their final width and
  // would spill past the growing card, but sticky header chrome must not be
  // clipped at rest. Set on the node so it lands in the animation's first frame.
  const node = useRef<HTMLElement | null>(null);
  return (
    <LayoutKey.Provider value={layoutKey}>
      <motion.section
        {...props}
        ref={(element: HTMLElement | null) => {
          node.current = element;
          if (typeof ref === "function") ref(element);
          else if (ref) ref.current = element;
        }}
        layout
        layoutDependency={layoutKey}
        transition={{ layout: REFLOW }}
        onLayoutAnimationStart={() =>
          node.current?.setAttribute("data-moving", "")
        }
        onLayoutAnimationComplete={() =>
          node.current?.removeAttribute("data-moving")
        }
        style={{ borderRadius: 12, ...style }}
        className={cn("data-[moving]:overflow-clip", className)}
      />
    </LayoutKey.Provider>
  );
}

/** A surface inside a `MotionCard` that resizes with it without distorting. */
export function MotionSurface(props: ComponentProps<typeof motion.div>) {
  return <motion.div {...props} layout {...useLayoutProps()} />;
}

/** Card content that slides to its new position at its final size. */
export function Glide(props: ComponentProps<typeof motion.div>) {
  return <motion.div {...props} layout="position" {...useLayoutProps()} />;
}

export function CardReveal({
  open,
  onClosed,
  className,
  children,
}: {
  readonly open: boolean;
  readonly onClosed: () => void;
  readonly className?: string;
  readonly children: ReactNode;
}) {
  const reveal = useRevealMotion(REVEAL_DELAY);
  return (
    <AnimatePresence initial={false} onExitComplete={onClosed}>
      {open && (
        <motion.div
          key="body"
          {...reveal}
          className={cn("overflow-clip", className)}
        >
          {children}
        </motion.div>
      )}
    </AnimatePresence>
  );
}

/** Fades content in when it replaces other content inside an open card. */
export function FadeIn(props: ComponentProps<typeof motion.div>) {
  return (
    <motion.div
      initial={{ opacity: 0 }}
      animate={{ opacity: 1 }}
      transition={{ duration: 0.18, ease: "easeOut" }}
      {...props}
    />
  );
}
