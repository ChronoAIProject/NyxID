import { websiteNavProps } from "./navigation";
import { useWebsite } from "./website-context";
import { SiteLink } from "./site-link";
import {
  BLOG_MENU_ENABLED,
  NEWSLETTER_ENABLED,
  CONTACT_CHAT_ENABLED,
  STATUS_ENABLED,
} from "./release-config";
import { NewsletterSignup as Newsletter } from "./newsletter-signup";
import { ContactChat as Chat } from "./contact-chat";
import { FooterStatus as Status } from "./footer-status";
const Hidden = () => null;
// Build-time branches exclude dormant forms/adapters from default bundles.
export const NewsletterSignup = NEWSLETTER_ENABLED ? Newsletter : Hidden;
export const ContactChat = CONTACT_CHAT_ENABLED ? Chat : Hidden;
export const FooterStatus = STATUS_ENABLED ? Status : Hidden;

export function MenuBlogLink() {
  const s = useWebsite();
  return (
    <SiteLink
      {...websiteNavProps(s.route, "/blog")}
      data-nav="blog"
      href={s.siteHref("/blog")}
    >
      {s.language === "zh" ? "博客" : "Blog"}
    </SiteLink>
  );
}
export const BlogMenuLink = BLOG_MENU_ENABLED ? MenuBlogLink : Hidden;
