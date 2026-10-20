import { createContext, useContext, useEffect } from "react";

export const BreadcrumbLabelContext = createContext<{
  pathname: string;
  labels: Readonly<Record<string, string>>;
  setLabel: (pathname: string, label: string | null) => void;
  sections?: Readonly<Record<string, string>>;
  setSection?: (pathname: string, section: string | null) => void;
}>({
  pathname: "",
  labels: {},
  setLabel: () => {},
});

export function useBreadcrumbLabel(label: string | undefined | null, path?: string) {
  const { pathname, setLabel } = useContext(BreadcrumbLabelContext);
  const labelPath = path ?? pathname;
  const stableLabel = label ?? null;
  useEffect(() => {
    setLabel(labelPath, stableLabel);
    return () => setLabel(labelPath, null);
  }, [labelPath, stableLabel, setLabel]);
}

export function useBreadcrumbSection(section: string) {
  const { pathname, setSection } = useContext(BreadcrumbLabelContext);
  useEffect(() => {
    setSection?.(pathname, section);
    return () => setSection?.(pathname, null);
  }, [pathname, section, setSection]);
}
