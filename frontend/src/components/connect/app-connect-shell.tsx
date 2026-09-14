import { ShieldCheck } from "lucide-react";
import { ConnectShell } from "./connection-panels";

export function AppConnectShell({
  name,
  blurb,
  destination,
  children,
}: {
  readonly name: string;
  readonly blurb: string | null;
  readonly destination: string;
  readonly children: React.ReactNode;
}) {
  return (
    <ConnectShell>
      <header className="space-y-2">
        <h1 className="font-heading text-[22px] font-bold tracking-tight sm:text-[28px]">
          {name}
        </h1>
        <p className="text-[12px] text-muted-foreground">
          {blurb ?? "Connect the accounts this app needs to continue."}
        </p>
      </header>
      {children}
      <footer className="flex items-center justify-center gap-1.5 border-t border-border/50 pt-4 text-[11px] text-muted-foreground">
        <ShieldCheck className="size-3.5 shrink-0" aria-hidden="true" />
        <span>Secured by NyxID · {destination}</span>
      </footer>
    </ConnectShell>
  );
}
