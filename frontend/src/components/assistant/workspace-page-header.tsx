import { StickyTitle } from "@/components/ui/compact-title";
import { PageHeader } from "@/components/shared/page-header";
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
    <StickyTitle title={title} actions={actions} column={column}>
      <PageHeader data-workspace-header="" titleAs="h1" title={title} description={description} actions={actions} className={column ? "max-w-2xl" : undefined} />
    </StickyTitle>
  );
}
