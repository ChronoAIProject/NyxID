import type { ReactNode } from "react";
import { ServiceIcon } from "@/components/service-icon";
import { DialogDescription, DialogTitle } from "@/components/ui/dialog";

/**
 * Inline header for a step body inside `AddKeyDialog`.
 *
 * This renders the *real* Radix `DialogTitle` / `DialogDescription` rather
 * than a visual copy of them, so the dialog keeps its `aria-labelledby` /
 * `aria-describedby` wiring and the accessibility tree holds exactly one
 * heading. (An earlier pass rendered a plain `<h2>` here and left an
 * sr-only `DialogHeader` above — that put two same-named headings in the
 * a11y tree and screen readers announced the title twice.)
 *
 * Rendering the title inside the step body — instead of in a `DialogHeader`
 * separated from it by `DialogContent`'s `gap-4` — is what makes the modal
 * read as one continuous block. Every step body renders exactly one of
 * these; because only a single step is mounted at a time, the dialog always
 * has exactly one title.
 */
export function StepHeader({
  slug,
  title,
  description,
}: {
  /** Catalog slug for the brand glyph. Omitted for custom endpoints. */
  readonly slug?: string | null;
  readonly title: ReactNode;
  readonly description?: ReactNode;
}) {
  return (
    <div className="flex items-start gap-3">
      {slug && (
        <div className="mt-0.5 shrink-0">
          <ServiceIcon slug={slug} size="md" />
        </div>
      )}
      <div className="min-w-0 space-y-1">
        <DialogTitle className="leading-tight">{title}</DialogTitle>
        {description && <DialogDescription>{description}</DialogDescription>}
      </div>
    </div>
  );
}
