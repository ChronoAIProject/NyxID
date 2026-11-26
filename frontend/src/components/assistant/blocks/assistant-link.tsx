import { isHostedConnectLink } from "@/lib/assistant/hosted-connect-link";
import { useContext, type ReactNode } from "react";
import { buttonVariants } from "@/components/ui/button";
import { assistantModalLinkTarget } from "@/lib/assistant/assistant-link-target";
import { NyxBotHandoffContext } from "@/lib/assistant/nyxbot-handoff-context";

export function AssistantLink({ href, children }: {
  readonly href: string;
  readonly children: ReactNode;
}) {
  const connect = typeof window !== "undefined" && isHostedConnectLink(href, window.location.origin);
  const controller = useContext(NyxBotHandoffContext);
  const target = typeof window !== "undefined" ? assistantModalLinkTarget(href, window.location.origin) : null;
  const conversation = Boolean((controller?.open && target?.kind === "conversation") || target?.kind === "workspace");
  const label = typeof children === "string" ? children.trim() : "service";
  const connectLabel = /^connect\b/i.test(label)
    ? label
    : `Connect ${label.startsWith("http") ? "service" : label}`;
  return (
    <a
      href={href}
      target={href.startsWith("#") || conversation ? undefined : "_blank"}
      rel="noopener noreferrer"
      className={connect
        ? buttonVariants({ variant: "default", size: "sm", className: "h-auto max-w-full whitespace-normal py-1 text-center" })
        : "text-nyx-secondary-400 underline decoration-nyx-secondary-400/40 " +
          "underline-offset-2 hover:decoration-nyx-secondary-400"}
    >
      {connect ? connectLabel : children}
    </a>
  );
}
