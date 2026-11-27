import { ArrowUpRight, BookOpen, Mail } from "lucide-react";
import type { ReactNode } from "react";
import { ContactSection } from "./contact";
import { contactEmailHref } from "./contact-mailto";
import { websiteConfig } from "./config";
import { ContactChat } from "./optional-features";
import { SiteLink } from "./site-link";
import { StoryPage } from "./story-page";
import { useWebsite } from "./website-context";
import "./commercial-story.css";

const DISCORD_URL = "https://discord.gg/QMvcs8UQBW";

function DiscordMark() {
  return (
    <svg viewBox="0 0 24 24" aria-hidden="true">
      <path d="M20.317 4.37a19.791 19.791 0 0 0-4.885-1.515.074.074 0 0 0-.079.037c-.21.375-.444.864-.608 1.25a18.27 18.27 0 0 0-5.487 0 12.64 12.64 0 0 0-.617-1.25.077.077 0 0 0-.079-.037A19.736 19.736 0 0 0 3.677 4.37a.07.07 0 0 0-.032.027C.533 9.046-.32 13.58.099 18.057a.082.082 0 0 0 .031.057 19.9 19.9 0 0 0 5.993 3.03.078.078 0 0 0 .084-.028c.462-.63.874-1.295 1.226-1.994a.076.076 0 0 0-.041-.106 13.107 13.107 0 0 1-1.872-.892.077.077 0 0 1-.008-.128 10.2 10.2 0 0 0 .372-.292.074.074 0 0 1 .077-.01c3.928 1.793 8.18 1.793 12.062 0a.074.074 0 0 1 .078.01c.12.098.246.198.373.292a.077.077 0 0 1-.006.127 12.299 12.299 0 0 1-1.873.892.077.077 0 0 0-.041.107c.36.698.772 1.362 1.225 1.993a.076.076 0 0 0 .084.028 19.839 19.839 0 0 0 6.002-3.03.077.077 0 0 0 .032-.054c.5-5.177-.838-9.674-3.549-13.66a.061.061 0 0 0-.031-.03zM8.02 15.33c-1.183 0-2.157-1.085-2.157-2.419 0-1.333.956-2.419 2.157-2.419 1.21 0 2.176 1.095 2.157 2.42 0 1.333-.956 2.418-2.157 2.418zm7.975 0c-1.183 0-2.157-1.085-2.157-2.419 0-1.333.956-2.419 2.157-2.419 1.21 0 2.176 1.095 2.157 2.42 0 1.333-.947 2.418-2.157 2.418z" />
    </svg>
  );
}

function Channel({
  href,
  icon,
  title,
  detail,
  external = false,
}: {
  href: string;
  icon: ReactNode;
  title: string;
  detail: string;
  external?: boolean;
}) {
  const content = (
    <>
      <span className="stc-ch-ic">{icon}</span>
      <span>
        <b>{title}</b>
        <small>{detail}</small>
      </span>
      <ArrowUpRight size={18} aria-hidden="true" />
    </>
  );
  return (
    <li>
      {external ? (
        <a href={href} target="_blank" rel="noopener noreferrer">
          {content}
        </a>
      ) : (
        <SiteLink href={href}>{content}</SiteLink>
      )}
    </li>
  );
}

/* Contact: why to write and the other ways to reach the team on the left; the
   existing form (mail-app handoff, unchanged) and the optional chat on the right. */
export function ContactStoryPage() {
  const s = useWebsite();
  const zh = s.language === "zh";
  return (
    <StoryPage className="st-commercial stc-contact">
      <section aria-labelledby="contact-title">
        <div className="st-wrap stc-contact-grid">
          <div className="stc-contact-intro">
            <h1 id="contact-title">{zh ? "联系我们" : "Talk to us"}</h1>
            <p className="st-lead">
              {zh
                ? "关于套餐、自托管或设置有疑问？选一种最方便的方式。"
                : "Questions about bundles, self-hosting or your setup? Pick whichever is easiest."}
            </p>
            <ul className="stc-channels">
              <Channel
                href={DISCORD_URL}
                external
                icon={<DiscordMark />}
                title={zh ? "在 Discord 和我们聊" : "Chat with us on Discord"}
                detail={zh ? "与团队和社区交流。" : "Talk to the team and the community."}
              />
              <Channel
                href={contactEmailHref(websiteConfig.contactEmail)}
                icon={<Mail aria-hidden="true" />}
                title={zh ? "直接发邮件" : "Email the team"}
                detail={websiteConfig.contactEmail}
              />
              <Channel
                href={s.siteHref("/docs")}
                icon={<BookOpen aria-hidden="true" />}
                title={zh ? "先看文档" : "Read the docs"}
                detail={
                  zh
                    ? "面向 AI 智能体、Web 应用和命令行的指南。"
                    : "Guides for AI agents, web apps and the CLI."
                }
              />
            </ul>
          </div>
          <div className="stc-contact-form">
            <ContactSection>
              <ContactChat />
            </ContactSection>
          </div>
        </div>
      </section>
    </StoryPage>
  );
}
