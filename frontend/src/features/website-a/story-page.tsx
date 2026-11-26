import { useId, type ReactNode } from "react";
import { ArrowRight, Check, ChevronDown, KeyRound, LockKeyhole } from "lucide-react";
import { SiteLink } from "./site-link";
import { useWebsite } from "./website-context";
import "./story-page.css";

export function StoryPage({ children, className = "" }: { children: ReactNode; className?: string }) {
  return <div className={`st-page ${className}`}>{children}</div>;
}

export function StoryHero({ eyebrow, title, description, children, visual }: { eyebrow?: ReactNode; title: ReactNode; description: ReactNode; children?: ReactNode; visual?: ReactNode }) {
  return <section className={`st-hero${visual ? " st-hero-split" : ""}`}>
    <div className="st-wrap st-hero-grid">
      <div className="st-hero-copy">
        {eyebrow && <p className="st-eyebrow">{eyebrow}</p>}
        <h1>{title}</h1>
        <p className="st-lead">{description}</p>
        {children && <div className="st-hero-actions">{children}</div>}
      </div>
      {visual && <div className="st-hero-visual">{visual}</div>}
    </div>
  </section>;
}

export function StorySection({ id, label, title, description, children, className = "" }: { id?: string; label?: ReactNode; title?: ReactNode; description?: ReactNode; children: ReactNode; className?: string }) {
  const generated = useId();
  const titleId = `${id ?? generated}-title`;
  return <section id={id} className={`st-section ${className}`} aria-labelledby={title ? titleId : undefined}>
    <div className="st-wrap">
      {(title || description || label) && <div className="st-section-head">
        {label && <p className="st-eyebrow">{label}</p>}
        {title && <h2 id={titleId}>{title}</h2>}
        {description && <p className="st-lead">{description}</p>}
      </div>}
      {children}
    </div>
  </section>;
}

export function StorySplit({ children, reverse = false, align = "center", className = "" }: { children: ReactNode; reverse?: boolean; align?: "center" | "start"; className?: string }) {
  return <div className={`st-split${reverse ? " st-split-reverse" : ""}${align === "start" ? " st-split-start" : ""} ${className}`}>{children}</div>;
}

export function StoryLink({ href, children, primary = false }: { href: string; children: ReactNode; primary?: boolean }) {
  return <SiteLink className={primary ? "st-button" : "st-link"} href={href}>{children}<ArrowRight size={17} aria-hidden="true" /></SiteLink>;
}

export function StoryNext({ title, description, href, label, secondaryHref, secondaryLabel }: { title: ReactNode; description?: ReactNode; href: string; label: ReactNode; secondaryHref?: string; secondaryLabel?: ReactNode }) {
  return <section className="st-next"><div className="st-wrap"><div className="st-next-inner">
    <div><h2>{title}</h2>{description && <p>{description}</p>}</div>
    <div className="st-actions"><StoryLink href={href}>{label}</StoryLink>{secondaryHref && secondaryLabel && <StoryLink href={secondaryHref}>{secondaryLabel}</StoryLink>}</div>
  </div></div></section>;
}

export function StoryDetails({ summary, children, open }: { summary: ReactNode; children: ReactNode; open?: boolean }) {
  return <details className="st-details" open={open}><summary className="list-none">{summary}<ChevronDown size={18} aria-hidden="true" /></summary><div className="st-details-body">{children}</div></details>;
}

export function AppPill({ name, icon, soon = false }: { name: string; icon?: string; soon?: boolean }) {
  const { language } = useWebsite();
  return <span className="st-app">{icon && <img src={`/landing/nyx/icons/${icon}.svg`} width="20" height="20" alt="" />}<span>{name}</span>{soon && <small>{language === "zh" ? "即将推出" : "Coming soon"}</small>}</span>;
}

export function PermissionReceipt({ compact = false }: { compact?: boolean }) {
  const zh = useWebsite().language === "zh";
  return <div className={`st-receipt st-permissions${compact ? " st-compact" : ""}`}>
    <div className="st-receipt-top"><KeyRound size={20} aria-hidden="true" /><span>{zh ? "餐厅助手" : "Restaurant assistant"}</span><small>{zh ? "权限示例" : "Example permissions"}</small></div>
    <h3>{zh ? "只开放这份工作所需的权限。" : "Just enough access for the job."}</h3>
    <div className="st-permission-row"><span>{zh ? "查看空位" : "See free times"}</span><span className="st-status st-allowed"><Check size={12} aria-hidden="true" />{zh ? "允许" : "Allowed"}</span></div>
    <div className="st-permission-row"><span>{zh ? "创建预订" : "Create a booking"}</span><span className="st-status st-allowed">{zh ? "允许" : "Allowed"}</span></div>
    <div className="st-permission-row"><span>{zh ? "取消预订" : "Cancel a booking"}</span><span className="st-status st-ask">{zh ? "先询问" : "Ask first"}</span></div>
    {!compact && <div className="st-permission-row"><span>{zh ? "收取订金" : "Charge a deposit"}</span><span className="st-status st-blocked">{zh ? "不允许" : "Not allowed"}</span></div>}
    <p className="st-receipt-note"><LockKeyhole size={14} aria-hidden="true" />{zh ? "真正的登录凭据留在 NyxID。" : "The real login stays with NyxID."}</p>
  </div>;
}
