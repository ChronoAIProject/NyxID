import { StickyTitle } from "@/components/ui/compact-title";
import type { ReactNode } from "react";
import "./workspace-pages.css";

export function WorkspacePageHeader({
  title,
  description,
  actions,
  column = false,
}: {
  readonly column?: boolean;
  readonly title: string;
  readonly description: string;
  readonly actions?: ReactNode;
}) {
  return (
    <StickyTitle title={title} actions={actions} column={column}><header
      data-workspace-header=""
      className={`flex flex-col gap-3 sm:flex-row sm:items-start sm:justify-between sm:gap-4${column ? " max-w-2xl" : ""}`}
    >
      <div className="min-w-0 space-y-2">
        <h1 className="text-22 font-bold leading-tight tracking-tight sm:text-28">
          {title}
        </h1>
        <p className="max-w-2xl text-12 text-muted-foreground">{description}</p>
      </div>
      {actions ? (
        <div data-title-actions="" className="flex shrink-0 flex-wrap items-center gap-2">
          {actions}
        </div>
      ) : null}
    </header></StickyTitle>
  );
}
