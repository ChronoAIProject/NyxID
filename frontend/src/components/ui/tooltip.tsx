import * as React from "react";
import * as TooltipPrimitive from "@radix-ui/react-tooltip";
import { OverlayLayer } from "@/components/ui/overlay-layer";
import { useOverlayLayer } from "@/lib/overlay-layer";
import { cn } from "@/lib/utils";

const TooltipProvider = TooltipPrimitive.Provider;
const Tooltip = TooltipPrimitive.Root;
const TooltipTrigger = TooltipPrimitive.Trigger;

const TooltipContent = React.forwardRef<
  React.ComponentRef<typeof TooltipPrimitive.Content>,
  React.ComponentPropsWithoutRef<typeof TooltipPrimitive.Content>
>(({ className, sideOffset = 4, style, ...props }, ref) => {
  const layer = Math.max(1000, useOverlayLayer());
  return (
  <OverlayLayer layer={layer}>
  <TooltipPrimitive.Portal>
    <TooltipPrimitive.Content
      ref={ref}
      sideOffset={sideOffset}
      className={cn(
        "z-[100] overflow-hidden rounded-[6px] bg-muted px-3 py-1.5 text-xs text-foreground shadow-lg shadow-primary/5",
          "data-[state=delayed-open]:animate-in data-[state=instant-open]:animate-in",
          "data-[state=closed]:animate-out",
          "data-[state=delayed-open]:fade-in-0 data-[state=instant-open]:fade-in-0",
          "data-[state=closed]:fade-out-0",
          "data-[state=delayed-open]:zoom-in-95 data-[state=instant-open]:zoom-in-95",
          "data-[state=closed]:zoom-out-95",
          "data-[side=bottom]:slide-in-from-top-2 data-[side=left]:slide-in-from-right-2 data-[side=right]:slide-in-from-left-2 data-[side=top]:slide-in-from-bottom-2",
        className,
      )}
      {...props}
      style={{ ...style, zIndex: layer }}
    />
  </TooltipPrimitive.Portal>
  </OverlayLayer>
  );
});
TooltipContent.displayName = TooltipPrimitive.Content.displayName;

export { Tooltip, TooltipTrigger, TooltipContent, TooltipProvider };
