import * as React from "react";
import * as TabsPrimitive from "@radix-ui/react-tabs";
import { cn } from "@/lib/utils";

const Tabs = TabsPrimitive.Root;

const TabsList = React.forwardRef<
  React.ComponentRef<typeof TabsPrimitive.List>,
  React.ComponentPropsWithoutRef<typeof TabsPrimitive.List>
>(({ className, children, ...props }, ref) => {
  const listRef = React.useRef<HTMLDivElement | null>(null);
  const [indicator, setIndicator] = React.useState({ left: 0, width: 0 });
  const [hasActive, setHasActive] = React.useState(false);

  const updateIndicator = React.useCallback(() => {
    const list = listRef.current;
    if (!list) return;
    const active = list.querySelector<HTMLElement>("[data-state=active]");
    if (active) {
      const listRect = list.getBoundingClientRect();
      const activeRect = active.getBoundingClientRect();
      setIndicator({
        left: activeRect.left - listRect.left + list.scrollLeft,
        width: activeRect.width,
      });
      setHasActive(true);
    }
  }, []);

  React.useEffect(() => {
    updateIndicator();
    const list = listRef.current;
    if (!list) return;
    const observer = new MutationObserver(updateIndicator);
    observer.observe(list, { attributes: true, subtree: true, attributeFilter: ["data-state"] });
    list.addEventListener("scroll", updateIndicator);
    return () => {
      observer.disconnect();
      list.removeEventListener("scroll", updateIndicator);
    };
  }, [updateIndicator]);

  return (
    <TabsPrimitive.List
      ref={(node) => {
        listRef.current = node;
        if (typeof ref === "function") ref(node);
        else if (ref) ref.current = node;
      }}
      className={cn(
        "relative flex h-8 items-center gap-1 border-b border-border bg-transparent p-0 text-muted-foreground overflow-x-auto overflow-y-hidden scrollbar-none",
        className,
      )}
      {...props}
    >
      {children}
      {hasActive && (
        <span
          className="absolute bottom-0 h-[2px] bg-primary transition-all duration-300 ease-out"
          style={{ left: indicator.left, width: indicator.width }}
        />
      )}
    </TabsPrimitive.List>
  );
});
TabsList.displayName = TabsPrimitive.List.displayName;

const TabsTrigger = React.forwardRef<
  React.ComponentRef<typeof TabsPrimitive.Trigger>,
  React.ComponentPropsWithoutRef<typeof TabsPrimitive.Trigger>
>(({ className, ...props }, ref) => (
  <TabsPrimitive.Trigger
    ref={ref}
    className={cn(
      "inline-flex shrink-0 items-center justify-center whitespace-nowrap px-3 py-2 text-[12px] font-normal text-text-tertiary transition-colors duration-300 focus-visible:outline-none disabled:pointer-events-none disabled:opacity-50 -mb-px data-[state=active]:text-foreground data-[state=active]:font-medium",
      className,
    )}
    {...props}
  />
));
TabsTrigger.displayName = TabsPrimitive.Trigger.displayName;

const TabsContent = React.forwardRef<
  React.ComponentRef<typeof TabsPrimitive.Content>,
  React.ComponentPropsWithoutRef<typeof TabsPrimitive.Content>
>(({ className, ...props }, ref) => (
  <TabsPrimitive.Content
    ref={ref}
    className={cn(
      "mt-3 focus-visible:outline-none",
      className,
    )}
    {...props}
  />
));
TabsContent.displayName = TabsPrimitive.Content.displayName;

export { Tabs, TabsList, TabsTrigger, TabsContent };
