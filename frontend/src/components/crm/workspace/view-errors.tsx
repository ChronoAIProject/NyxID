import { AlertCircle } from "lucide-react";
import type { CrmValidationError } from "@/lib/crm/views/schema";

export function CrmViewErrors({
  errors,
  title = "This view needs attention",
}: {
  readonly errors: readonly CrmValidationError[];
  readonly title?: string;
}) {
  return (
    <div
      role="alert"
      className="rounded-xl border border-destructive/20 bg-destructive/[0.04] p-4"
    >
      <div className="flex items-start gap-3">
        <div className="flex h-9 w-9 shrink-0 items-center justify-center rounded-lg bg-destructive/10">
          <AlertCircle className="size-4 text-destructive" />
        </div>
        <div className="min-w-0 flex-1">
          <h3 className="type-secondary-strong text-destructive">{title}</h3>
          <ul className="mt-2 space-y-1.5">
            {errors.map((error, index) => (
              <li
                key={`${error.path}-${String(index)}`}
                className="text-12 text-destructive"
              >
                <code className="font-mono type-meta">{error.path}</code>
                <span aria-hidden="true">: </span>
                {error.message}
              </li>
            ))}
          </ul>
        </div>
      </div>
    </div>
  );
}
