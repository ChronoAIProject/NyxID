import { Link } from "@tanstack/react-router";
import { ArrowLeft, ChevronRight } from "lucide-react";

export interface BreadcrumbItem {
  readonly label: string;
  readonly to?: string;
}

interface BreadcrumbProps {
  readonly items: readonly BreadcrumbItem[];
}

/* ── NyxID Breadcrumb ── */
export function Breadcrumb({ items }: BreadcrumbProps) {
  const parentTo = items.length > 1 ? items[0]?.to : undefined;

  return (
    <nav aria-label="Breadcrumb" className="flex items-center gap-1.5 text-sm">
      {parentTo && (
        <Link
          to={parentTo}
          className="mr-1 flex h-7 w-7 items-center justify-center rounded-[6px] border border-white/[0.08] bg-white/[0.04] text-text-tertiary transition-all duration-200 hover:border-white/[0.15] hover:text-foreground"
        >
          <ArrowLeft className="h-3.5 w-3.5" />
        </Link>
      )}
      {items.map((item, index) => {
        const isLast = index === items.length - 1;

        return (
          <div key={item.label} className="flex items-center gap-1.5">
            {index > 0 && (
              <ChevronRight className="h-3.5 w-3.5 text-text-tertiary" />
            )}
            {item.to && !isLast ? (
              <Link
                to={item.to}
                className="text-text-tertiary transition-colors duration-300 hover:text-foreground"
              >
                {item.label}
              </Link>
            ) : (
              <span className="font-medium text-foreground">{item.label}</span>
            )}
          </div>
        );
      })}
    </nav>
  );
}
