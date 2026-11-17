import { Fragment, useLayoutEffect, useRef, type ReactNode } from "react";
import { subscribeAssistantIdentity } from "@/lib/assistant/identity";
import { useAuthStore } from "@/stores/auth-store";

/** Drop page-local messages, selections and portals before publishing another account. */
export function AccountBoundary({ children }: { children: ReactNode }) {
  const generation = useAuthStore((state) => state.accountGeneration);
  const verifying = useAuthStore((state) => state.isIdentityVerifying);
  const host = useRef<HTMLDivElement>(null);
  useLayoutEffect(
    () =>
      subscribeAssistantIdentity((_next, previous) => {
        if (previous)
          document.documentElement.setAttribute("data-account-transition", "");
      }),
    [],
  );
  useLayoutEffect(() => {
    document.documentElement.removeAttribute("data-account-transition");
  }, [generation]);
  useLayoutEffect(
    () =>
      useAuthStore.subscribe((state) => {
        // Browser lifecycle signals hide synchronously, before the next React commit.
        if (host.current) {
          host.current.hidden = state.isIdentityVerifying;
          host.current.style.display = state.isIdentityVerifying
            ? "none"
            : "contents";
        }
      }),
    [],
  );
  return (
    <div
      ref={host}
      hidden={verifying}
      style={{ display: verifying ? "none" : "contents" }}
    >
      <Fragment key={generation}>{children}</Fragment>
    </div>
  );
}
