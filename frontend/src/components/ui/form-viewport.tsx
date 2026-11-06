import { useState, type ComponentProps } from "react";
import { FormViewportContext } from "@/lib/form-viewport";
/** Menus collide with the editing viewport, leaving its save actions available. */
export function FormViewport({ children, ...props }: ComponentProps<"div">) {
  const [element, setElement] = useState<HTMLDivElement | null>(null);
  return <div {...props} ref={setElement} data-form-viewport=""><FormViewportContext value={element}>{children}</FormViewportContext></div>;
}
