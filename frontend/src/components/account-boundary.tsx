import { Fragment, useLayoutEffect, type ReactNode } from "react";
import { subscribeAssistantIdentity } from "@/lib/assistant/identity";
import { useAuthStore } from "@/stores/auth-store";

/** Every consumer of a replaced QueryClient remounts, including Root's observers. */
export function AccountBoundary({ children }: { children: ReactNode }) {
  const generation = useAuthStore((state) => state.accountGeneration);
  useLayoutEffect(() => subscribeAssistantIdentity((_next, previous) => {
    if (previous) document.documentElement.setAttribute("data-account-transition", "");
  }), []);
  useLayoutEffect(() => {
    document.documentElement.removeAttribute("data-account-transition");
  }, [generation]);
  return <Fragment key={generation}>{children}</Fragment>;
}
