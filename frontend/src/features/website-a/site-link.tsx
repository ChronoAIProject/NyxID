import { forwardRef, useContext } from "react";
import type { AnchorHTMLAttributes } from "react";
import { Link } from "@tanstack/react-router";
import { PublicSearchContext, parsePublicSearch } from "../public-site/search";
import { isContentPath } from "../public-site/routes";

/** Router owns only public content. Protocol and app links retain document navigation. */
export const SiteLink = forwardRef<
  HTMLAnchorElement,
  AnchorHTMLAttributes<HTMLAnchorElement>
>(function SiteLink({ href = "", ...props }, ref) {
  const publicSearch = useContext(PublicSearchContext);
  const url =
    href.startsWith("/") && !href.startsWith("//")
      ? new URL(href, "https://nyx.chrono-ai.fun")
      : null;
  if (
    !url ||
    !isContentPath(url.pathname) ||
    (!publicSearch && Boolean(url.search))
  )
    return <a {...props} href={href} ref={ref} />;
  return (
    <Link
      {...props}
      ref={ref}
      to={url.pathname as never}
      search={(publicSearch ? parsePublicSearch(url.search) : {}) as never}
      hash={url.hash.slice(1)}
      preload={false}
    />
  );
});
