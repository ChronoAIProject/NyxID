import { Check, KeyRound, MessageCircle, Utensils, Newspaper, Phone, Search, CalendarDays, Monitor, Sparkles } from "lucide-react";
import type { LucideIcon } from "lucide-react";
import { useWebsite } from "./website-context";
import { SiteLink } from "./site-link";
import { SolutionActivation } from "./solution-activation";
import { websiteRelease } from "./release-config";
import { AppPill, StoryDetails, StoryHero, StoryLink, StoryNext, StoryPage, StorySection, StorySplit } from "./story-page";
import copy from "./solution-story-copy.json";
import pricing from "./pricing-copy.json";
import "./solution-story-pages.css";

const icons: Record<string,LucideIcon> = { "restaurant-manager":Utensils, "help-desk":MessageCircle, "appointment-booker":CalendarDays, "callback-queue":Phone, "stock-analyst":Search, "news-aggregator":Newspaper, "desk-researcher":Search, "x-lead-catcher":MessageCircle, "back-office-clerk":Monitor };
const appIcons: Record<string,string> = {
  Telegram:"telegram", X:"x", "X DMs":"x", "X 私信":"x", "X (Twitter)":"x", "X（推特）":"x",
  "Google Calendar":"google-calendar", "Google 日历":"google-calendar",
  "Google Sheets":"google-sheets", "Google 表格":"google-sheets",
  "Google Docs":"google-docs", "Google 文档":"google-docs",
  "Google Drive":"google-drive", "Google 云端硬盘":"google-drive",
  Twilio:"twilio", ElevenLabs:"elevenlabs", Gmail:"gmail", Notion:"notion",
};

import { SolutionsStoryPage } from "./solutions-index";
export { SolutionsStoryPage };

// A step whose approval slot is empty ("Not needed") is data for the index, not a step to show.
const isEmptyStep = (title: string) => title === "Not needed" || title === "不需要";

/* One job, in its own words: the conversation, its workflow, what it needs and may do, its questions,
   then the next job. Headings come from the job's copy, so nine pages never share a heading. */
export function SolutionStoryPage() {
  const s=useWebsite(), zh=s.language==="zh", jobs=copy[s.language], slug=s.route.split("/").at(-1);
  const at=jobs.findIndex(job=>job.slug===slug), job=jobs[at];
  if(!job) return <SolutionsStoryPage />;
  const next=jobs[(at+1)%jobs.length] ?? job;
  const Icon=icons[job.slug]??Sparkles;
  const available = Boolean(websiteRelease.solutions[job.slug as keyof typeof websiteRelease.solutions]);
  const steps=job.flow.filter(step=>!isEmptyStep(step.title));
  return <StoryPage className="st-solution-detail">
    <StoryHero eyebrow={<span><SiteLink href={s.siteHref("/solutions")}>{zh ? "工作场景" : "Solutions"}</SiteLink><span className="st-breadcrumb-slash">/</span>{job.categoryLabel}</span>} title={job.title} description={job.description} visual={<div className="st-job-chat"><div className="st-job-chat-head"><Icon size={22} /><b>{job.title}</b><small>{zh ? "示例" : "Example"}</small></div><div className="msgs">{job.messages.map((message,index)=><div key={index} className={`m ${message.kind}`}>{message.text}</div>)}</div></div>}>
      {available ? <SolutionActivation /> : <><p className="st-availability"><strong>{zh ? "暂未开放。" : "Not available yet."}</strong> {zh ? "和我们聊聊如何为你设置。" : "Talk to us about setting it up."}</p><StoryLink primary href={s.siteHref("/contact")}>{zh ? "和我们聊聊" : "Talk it through"}</StoryLink></>}
      <div className="st-job-apps st-apps">{job.apps.filter(app=>!job.permissions.some(p=>p.service===app&&p.kind==="b-no")).map(app=><AppPill key={app} name={app} icon={appIcons[app]} />)}</div>
    </StoryHero>
    <StorySection id="workflow" title={job.workflowHeading}><ol className={`st-job-flow st-job-flow-${steps.length}`}>{steps.map((step,index)=><li key={step.label}><span className="st-step-number">{String(index+1).padStart(2,"0")} / {step.label}</span><h3>{step.title}</h3><p>{step.text}</p></li>)}</ol></StorySection>
    <StorySection id="before-it-starts"><StorySplit align="start" className="st-job-before">
      <div>
        <h2>{job.setupHeading}</h2>
        <ul className="st-setup-list">{job.setup.map(item=><li key={item}><Check size={16} aria-hidden="true" /><span>{item}</span></li>)}</ul>
        {!available && <StoryDetails summary={zh ? "方案开放情况" : "Setup availability"}><p>{zh ? "这个工作流程暂未开放一键启用，可以先联系我们讨论你的设置。" : "This workflow is not available for one-click setup yet. Talk to us about your setup in the meantime."}</p><SolutionActivation /></StoryDetails>}
        <p className="st-job-price-note">{pricing[s.language].description} <SiteLink className="st-inline-link" href={s.siteHref("/pricing")}>{zh ? "价格" : "Pricing"}</SiteLink></p>
      </div>
      <figure className="st-job-permissions">
        <div className="st-receipt"><div className="st-receipt-top"><KeyRound size={18} /><span>{job.title}</span></div>{job.permissions.map((p,index)=><div className="st-permission-row" key={index}><span>{p.service}<small className="st-permission-action">{p.action}</small></span><span className={`st-status ${p.kind==="b-no"?"st-blocked":p.kind==="b-ask"?"st-ask":p.kind==="b-ro"?"st-read-only":"st-allowed"}`}>{p.status}</span></div>)}</div>
        <figcaption>{zh ? "这份工作建议的权限，开始前可以逐项调整。" : "Proposed permissions for this job. You can change each one before it starts."} <SiteLink className="st-inline-link" href={s.siteHref("/security")}>{zh ? "权限如何运作" : "How permissions work"}</SiteLink></figcaption>
      </figure>
    </StorySplit></StorySection>
    {job.faqs.length>0&&<StorySection id="questions"><StorySplit align="start"><div><h2>{job.faqHeading}</h2></div><div>{job.faqs.map(faq=><StoryDetails key={faq.question} summary={faq.question}><p>{faq.answer}</p></StoryDetails>)}</div></StorySplit></StorySection>}
    <StoryNext title={zh ? `下一个场景：${next.title}` : `Next: ${next.title}`} description={next.description} href={s.siteHref(`/solutions/${next.slug}`)} label={zh ? "查看流程" : "See the workflow"} secondaryHref={s.siteHref("/solutions")} secondaryLabel={zh ? "全部场景" : "All solutions"} />
  </StoryPage>;
}
