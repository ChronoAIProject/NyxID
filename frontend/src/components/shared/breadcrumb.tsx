import { Link } from "@tanstack/react-router";
import { ChevronRight } from "lucide-react";

export interface BreadcrumbItem {
  readonly label: string;
  readonly to?: string;
}

interface BreadcrumbProps {
  readonly items: readonly BreadcrumbItem[];
}

/* ── VoidPortal Breadcrumb ── */
export function Breadcrumb({ items }: BreadcrumbProps) {
  return (
    <nav aria-label="Breadcrumb" className="flex items-center gap-1 text-sm">
      {items.map((item, index) => {
        const isLast = index === items.length - 1;

        return (
          <div key={item.label} className="flex items-center gap-1">
            {index > 0 && (
              <ChevronRight className="h-3.5 w-3.5 text-text-tertiary" />
            )}
            {item.to && !isLast ? (
              <Link
                to={item.to}
                className="text-text-tertiary transition-colors hover:text-foreground"
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
