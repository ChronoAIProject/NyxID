import { useParams } from "@tanstack/react-router";
import { BlogDetailPage as LegacyBlogDetailPage } from "@/features/blog/blog-detail-page";
import { WebsitePage } from "@/features/website-a/website-page";

const legacySlugs = new Set([
  "push-approvals-walkthrough",
  "aes-envelope-encryption",
  "oauth-to-delegated-access",
  "designing-for-the-threshold",
  "audit-trail-postmortem",
]);

export function BlogDetailPage() {
  const { slug } = useParams({ from: "/blog/$slug" });
  return legacySlugs.has(slug) ? <LegacyBlogDetailPage /> : <WebsitePage />;
}
