import { useState } from "react";
import { ActionCard } from "@/components/assistant/blocks/action-card";
import { ApprovalCard } from "@/components/assistant/blocks/approval-card";
import { ThemeToggle } from "@/components/dashboard/theme-toggle";
import { PageHeader } from "@/components/shared/page-header";
import { useApplyTheme } from "@/hooks/use-theme";
import {
  actionCardGalleryFixtures,
  type ActionCardGallerySpecimen,
  type ApprovalCardGallerySpecimen,
} from "@/lib/assistant/gallery-fixtures";
import type { ActionReport } from "@/schemas/assistant-actions";
import type { ActionCardContentBlock } from "@/types/assistant";

function SpecimenCaption({ children }: { readonly children: string }) {
  return (
    <p className="font-mono text-[10px] uppercase tracking-[1px] text-text-tertiary">
      {children}
    </p>
  );
}

function SectionHeading({
  title,
  description,
}: {
  readonly title: string;
  readonly description: string;
}) {
  return (
    <div className="space-y-1">
      <h2 className="text-[15px] font-semibold text-foreground">{title}</h2>
      <p className="text-[12px] text-muted-foreground">{description}</p>
    </div>
  );
}

function ActionSpecimen({
  specimen,
}: {
  readonly specimen: ActionCardGallerySpecimen;
}) {
  return (
    <div className="min-w-0 space-y-2">
      <SpecimenCaption>{specimen.caption}</SpecimenCaption>
      <ActionCard
        block={specimen.block}
        onProgress={() => undefined}
        onResolve={() => undefined}
      />
    </div>
  );
}

function ApprovalSpecimen({
  specimen,
}: {
  readonly specimen: ApprovalCardGallerySpecimen;
}) {
  return (
    <div className="min-w-0 space-y-2">
      <SpecimenCaption>{specimen.caption}</SpecimenCaption>
      <ApprovalCard block={specimen.block} onDecide={() => undefined} />
    </div>
  );
}

function resolvedLiveBlock(
  block: ActionCardContentBlock,
  report: ActionReport,
): ActionCardContentBlock {
  if (report.disposition === "completed") {
    return {
      ...block,
      status: "completed",
      outcome_note:
        "GitHub is available to the assistant through service us_44.",
    };
  }
  return {
    ...block,
    status: "declined",
    outcome_note: "GitHub was not connected. No account access was granted.",
  };
}

function LiveWizard() {
  const fixture = actionCardGalleryFixtures.liveWizard;
  const [block, setBlock] = useState<ActionCardContentBlock>(fixture.block);

  function setProgress(blockId: string, inProgress: boolean) {
    setBlock((current) =>
      current.block_id === blockId &&
      (current.status === "pending" || current.status === "in_progress")
        ? { ...current, status: inProgress ? "in_progress" : "pending" }
        : current,
    );
  }

  function resolve(report: ActionReport) {
    setBlock((current) => resolvedLiveBlock(current, report));
  }

  return (
    <div className="max-w-[680px] space-y-2">
      <SpecimenCaption>{fixture.caption}</SpecimenCaption>
      <ActionCard block={block} onProgress={setProgress} onResolve={resolve} />
      <p className="text-[11px] leading-relaxed text-muted-foreground">
        For the complete conversation and automatic continuation, open{" "}
        <a
          href="/assistant?mock"
          className="font-mono text-nyx-secondary-400 hover:text-foreground"
        >
          /assistant?mock
        </a>
        .
      </p>
    </div>
  );
}

function GalleryHeader() {
  return (
    <PageHeader
      title="Assistant action cards"
      description="Shipped card states and the real connect journey, rendered from seeded mock data."
      actions={<ThemeToggle />}
    />
  );
}

export function AssistantActionCardsGalleryPage() {
  useApplyTheme();
  const mockMode = new URLSearchParams(window.location.search).has("mock");

  if (!mockMode) {
    return (
      <main className="min-h-dvh bg-background px-4 py-6 text-foreground sm:px-6 md:px-8 lg:px-10">
        <div className="mx-auto max-w-[1360px] space-y-8">
          <GalleryHeader />
          <p className="text-[12px] text-muted-foreground">
            This development gallery needs mock mode. Open{" "}
            <a
              href="/design/action-cards?mock"
              className="font-mono text-nyx-secondary-400 hover:text-foreground"
            >
              /design/action-cards?mock
            </a>
            .
          </p>
        </div>
      </main>
    );
  }

  return (
    <main className="min-h-dvh bg-background px-4 py-6 text-foreground sm:px-6 md:px-8 lg:px-10">
      <div className="mx-auto max-w-[1360px] space-y-10">
        <GalleryHeader />

        <section className="space-y-5">
          <SectionHeading
            title="Action card states"
            description="Six lifecycle states plus the custom endpoint and routed organization variants."
          />
          <div className="grid gap-x-6 gap-y-8 lg:grid-cols-2">
            {actionCardGalleryFixtures.actionCards.map((specimen) => (
              <ActionSpecimen key={specimen.caption} specimen={specimen} />
            ))}
          </div>
        </section>

        <section className="space-y-5">
          <SectionHeading
            title="Approval card states"
            description="Per-request and grant decisions across every shipped terminal state."
          />
          <div className="grid gap-x-6 gap-y-8 lg:grid-cols-2">
            {actionCardGalleryFixtures.approvalCards.map((specimen) => (
              <ApprovalSpecimen key={specimen.caption} specimen={specimen} />
            ))}
          </div>
        </section>

        <section className="space-y-5">
          <SectionHeading
            title="Live connect journey"
            description="The GitHub action opens the shipped Add Service dialog and resolves this card locally."
          />
          <LiveWizard />
        </section>
      </div>
    </main>
  );
}
