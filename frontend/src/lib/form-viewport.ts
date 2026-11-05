import { createContext, useLayoutEffect, useState, type CSSProperties } from "react";
export const FormViewportContext = createContext<HTMLElement | null>(null);
export function useVisualViewportBounds(): CSSProperties {
  const [bounds, setBounds] = useState<CSSProperties>({});
  useLayoutEffect(() => {
    const viewport = window.visualViewport;
    const mobile = window.matchMedia("(max-width: 767px)");
    function update() {
      setBounds(viewport && mobile.matches ? { top: viewport.offsetTop, bottom: Math.max(0, window.innerHeight - viewport.height - viewport.offsetTop), height: "auto", maxHeight: viewport.height } : {});
    }
    update();
    viewport?.addEventListener("resize", update);
    viewport?.addEventListener("scroll", update);
    mobile.addEventListener("change", update);
    return () => { viewport?.removeEventListener("resize", update); viewport?.removeEventListener("scroll", update); mobile.removeEventListener("change", update); };
  }, []);
  return bounds;
}
