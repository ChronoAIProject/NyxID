import { useFormState } from "react-hook-form";
import { firstNestedErrorMessage } from "@/lib/form-errors";
import type { ReactNode } from "react";
import { ApiError } from "@/lib/api-client";
import { Button } from "@/components/ui/button";
import { Label } from "@/components/ui/label";
import { ErrorBanner } from "@/components/shared/error-banner";
import {
  componentLabel,
  type RetailComponent,
} from "@/schemas/platform-models";

export function Field({
  title,
  children,
}: {
  readonly title: string;
  readonly children: ReactNode;
}) {
  return (
    <Label className="grid gap-2">
      {title}
      {children}
    </Label>
  );
}
export function SaveError({
  error,
  reload,
}: {
  readonly error: unknown;
  readonly reload?: () => void;
}) {
  if (!error) return null;
  const conflict = error instanceof ApiError && error.status === 409;
  return (
    <div role="alert" className="space-y-2">
      <ErrorBanner
        message={
          conflict
            ? "This model changed. Your edits have not been applied. Reload the current revision before saving again."
            : error instanceof Error
              ? error.message
              : "The change could not be saved."
        }
      />
      {reload && (
        <Button type="button" variant="outline" onClick={reload}>
          {conflict ? "Discard edits and reload" : "Reload current state"}
        </Button>
      )}
    </div>
  );
}
export function PriceRows({
  components,
}: {
  readonly components: RetailComponent[];
}) {
  return (
    <dl className="divide-y divide-border/30">
      {components.map((component, index) => (
        <div
          key={`${component.selector.kind}-${index}`}
          className="flex flex-wrap items-center justify-between gap-3 px-4 py-3"
        >
          <dt className="type-body text-muted-foreground">
            {componentLabel(component)}
          </dt>
          <dd className="type-body tabular-nums">
            {component.explicit_free
              ? "Free"
              : `${component.credits_per_unit} credits / unit`}
          </dd>
        </div>
      ))}
    </dl>
  );
}

/** Invalid forms disable Save, so show the validation reason before submit. */
export function EditErrors() {
  const { errors } = useFormState();
  const message = firstNestedErrorMessage(errors);
  return message ? (
    <p role="alert" className="type-validation-error text-destructive">
      {message}
    </p>
  ) : null;
}
