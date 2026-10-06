import { Link } from "@tanstack/react-router";
import { ChevronRight } from "lucide-react";
import type { StudioBreadcrumb } from "@/lib/studio-breadcrumbs";

function StudioBreadcrumbLink({
  crumb,
}: {
  readonly crumb: StudioBreadcrumb & { to: string };
}) {
  const { to, search } = crumb;
  const props = {
    className:
      "text-text-tertiary truncate transition-colors duration-200 hover:text-foreground",
    children: crumb.label,
  };
  const tab = search?.tab;
  switch (to) {
    case "/settings":
      return <Link {...props} to="/settings" search={{ tab }} />;
    case "/settings/consents":
      return <Link {...props} to="/settings/consents" search={{ tab }} />;
    case "/keys":
      return <Link {...props} to="/keys" search={{ tab }} />;
    case "/integration-guide":
      return <Link {...props} to="/integration-guide" search={{ tab }} />;
    case "/admin/credits":
      return <Link {...props} to="/admin/credits" search={{ tab }} />;
    case "/admin/invite-codes":
      return (
        <Link
          {...props}
          to="/admin/invite-codes"
          search={{ view: search?.view }}
        />
      );
    case "/billing":
      return (
        <Link
          {...props}
          to="/billing"
          search={{ tab: tab === "usage" ? "usage" : "billing" }}
        />
      );
    case "/admin/usage":
      return (
        <Link
          {...props}
          to="/admin/usage"
          search={{ tab: tab === "list" ? "list" : "dashboard" }}
        />
      );
  }
  if (/^\/orgs\/[^/]+$/.test(to)) {
    return (
      <Link
        {...props}
        to="/orgs/$orgId"
        params={{ orgId: decodeURIComponent(to.slice("/orgs/".length)) }}
        search={{ tab }}
      />
    );
  }
  if (/^\/keys\/[^/]+$/.test(to)) {
    return (
      <Link
        {...props}
        to="/keys/$keyId"
        params={{ keyId: decodeURIComponent(to.slice("/keys/".length)) }}
        search={{ tab }}
      />
    );
  }
  return <Link {...props} to={to} search={{}} />;
}

export function StudioBreadcrumbTrail({
  crumbs,
}: {
  readonly crumbs: readonly StudioBreadcrumb[];
}) {
  if (crumbs.length === 0) return null;

  return (
    <nav
      aria-label="Breadcrumb"
      className="hidden md:flex items-center gap-1 text-12 min-w-0"
    >
      {crumbs.map((crumb, i) => (
        <div
          key={`${crumb.to ?? "current"}-${i}`}
          className="flex items-center gap-1 min-w-0"
        >
          {i > 0 && (
            <ChevronRight
              aria-hidden="true"
              className="h-3 w-3 shrink-0 text-text-tertiary/60"
            />
          )}
          {crumb.to ? (
            <StudioBreadcrumbLink crumb={{ ...crumb, to: crumb.to }} />
          ) : (
            <span
              aria-current="page"
              className="text-muted-foreground truncate"
            >
              {crumb.label}
            </span>
          )}
        </div>
      ))}
    </nav>
  );
}
