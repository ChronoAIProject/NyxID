import { forwardRef } from "react";
import type { AnchorHTMLAttributes } from "react";
import { Link } from "@tanstack/react-router";
import { isContentPath } from "../public-site/routes";

/** Router owns only public content. Protocol and app links retain document navigation. */
export const SiteLink = forwardRef<
  HTMLAnchorElement,
  AnchorHTMLAttributes<HTMLAnchorElement>
>(function SiteLink({ href = "", ...props }, ref) {
  const url =
    href.startsWith("/") && !href.startsWith("//")
      ? new URL(href, "https://nyx.chrono-ai.fun")
      : null;
  if (
    !url ||
    !isContentPath(url.pathname) ||
    new Set(url.searchParams.keys()).size !==
      [...url.searchParams.keys()].length
  )
    return <a {...props} href={href} ref={ref} />;
  return (
    <Link
      {...props}
      ref={ref}
      to={url.pathname as never}
      search={Object.fromEntries(url.searchParams) as never}
      hash={url.hash.slice(1)}
      preload={false}
    />
  );
});
