import type { ReactNode } from "react";
// Direct file import (not the barrel) so the CLI-embedded wizard bundle,
// which renders this via `wizard-entry.tsx`, pulls in only this one icon
// instead of the whole empty-state set.
import { RoadBarrierIcon } from "@/components/icons/empty-state/road-barrier";

/**
 * Shared "not found" page. Presentational and router-agnostic on purpose:
 * it renders no navigation of its own so it can be used both in the
 * desktop app (TanStack Router — see `AppNotFound`) and in the standalone
 * CLI wizard bundle (`wizard-entry.tsx`), which has no router and would
 * crash on a `<Link>`. Callers pass the appropriate call-to-action via
 * `action`.
 *
 * Follows the empty-state visual language (DESIGN.md): a faint themed
 * illustration over a mono uppercase code label, a serif headline, and a
 * muted supporting line.
 */
export function NotFoundPage({
  code = "404",
  title = "Page not found",
  description = "The page you're looking for doesn't exist or may have moved.",
  action,
}: {
  readonly code?: string;
  readonly title?: string;
  readonly description?: ReactNode;
  readonly action?: ReactNode;
}) {
  return (
    <div className="flex min-h-[60vh] w-full flex-col items-center justify-center gap-1 px-6 py-12 text-center">
      <RoadBarrierIcon className="h-48 w-48 text-muted-foreground/30" />
      <p className="font-mono text-xs uppercase tracking-widest text-text-tertiary">
        {code}
      </p>
      <h1 className="mt-2 font-serif text-[28px] font-normal text-foreground">
        {title}
      </h1>
      <p className="mt-1 max-w-sm text-sm text-muted-foreground">
        {description}
      </p>
      {action ? <div className="mt-6">{action}</div> : null}
    </div>
  );
}
