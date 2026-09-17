import { isHostedConnectLink } from "@/lib/assistant/hosted-connect-link";
import type { ReactNode } from "react";
import { buttonVariants } from "@/components/ui/button";

export function AssistantLink({ href, children }: {
  readonly href: string;
  readonly children: ReactNode;
}) {
  const connect = typeof window !== "undefined" && isHostedConnectLink(href, window.location.origin);
  const label = typeof children === "string" ? children.trim() : "service";
  const connectLabel = /^connect\b/i.test(label)
    ? label
    : `Connect ${label.startsWith("http") ? "service" : label}`;
  return (
    <a
      href={href}
      target={href.startsWith("#") ? undefined : "_blank"}
      rel="noopener noreferrer"
      className={connect
        ? buttonVariants({ variant: "default", size: "sm" })
        : "text-nyx-secondary-400 underline decoration-nyx-secondary-400/40 " +
          "underline-offset-2 hover:decoration-nyx-secondary-400"}
    >
      {connect ? connectLabel : children}
    </a>
  );
}
