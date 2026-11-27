import { ArrowRight, LockKeyhole, Sparkles } from "lucide-react";
import { useWebsite } from "./website-context";
import { SiteLink } from "./site-link";
import { StoryHero, StoryNext, StoryPage, StorySection } from "./story-page";
import { WorkflowMarks, workflowMarks, ChannelMark } from "./solution-workflow-icons";
import { websiteRelease } from "./release-config";
import copy from "./solution-story-copy.json";
import "./solution-story-pages.css";

const isAvailable = (slug: string) => Boolean(websiteRelease.solutions[slug as keyof typeof websiteRelease.solutions]);

export function SolutionsStoryPage() {
  const s=useWebsite(), zh=s.language==="zh", jobs=copy[s.language];
  const categories=zh ? [["all","全部"],["hosp","接待与预约"],["support","客户服务"],["research","研究与资讯"],["sales","销售与营销"],["ops","日常运营"]] : [["all","All jobs"],["hosp","Hospitality"],["support","Customer support"],["research","Research & insights"],["sales","Sales & marketing"],["ops","Operations"]];
  // Availability is stated before the click, from the same release settings that gate activation.
  const ready=jobs.filter(job=>isAvailable(job.slug)).length;
  const availability = ready===jobs.length ? "" : ready===0
    ? (zh ? "这些工作暂未开放一键启用，我们可以和你一起设置。" : "None of these can be switched on yet; we can set one up with you.")
    : (zh ? "标有“可以设置”的工作现在即可启用，其余的我们可以和你一起设置。" : "Jobs marked Ready to set up can be switched on now; we can set up the others with you.");
  return <StoryPage className="st-solutions">
    <StoryHero title={zh ? "可以交给助手的工作" : "Jobs you can hand to an assistant"} description={<>{zh ? "每个场景都写明请求从哪里来、它在你的应用里做哪几步，以及它不能做什么。" : "Each one shows where the request comes from, the steps it takes in your apps and what it must not do."}{availability && <> {availability}</>}</>} />
    <StorySection id="choose-a-job">
      <div className="st-job-filters"><div id="catF" className="st-filter-pills" aria-label={zh ? "工作类别" : "Job category"}>{categories.map(([value,label])=><button type="button" data-v={value} key={value} className={s.filterClass("catF",value!)} aria-pressed={s.isFilterSelected("catF",value!)} onClick={()=>s.setFilter("catF",value!)}>{label}</button>)}</div><div id="chF" className="st-filter-pills" aria-label={zh ? "聊天渠道" : "Channel"}><span>{zh ? "渠道" : "Channel"}</span>{["Telegram","X","Phone","Computer"].map(channel=><button type="button" key={channel} className={s.filterClass("chF",channel)} aria-pressed={s.isFilterSelected("chF",channel)} onClick={()=>s.setFilter("chF",channel)}>{channel}</button>)}</div></div>
      {/* A grid of the landing page's job tickets: where the job starts, three steps of its workflow, its boundary. */}
      <div id="solGrid" className="st-job-grid">
        {jobs.map((job) => {
          const steps = [job.flow[0], job.flow[2], job.flow[4]];
          return (
            <SiteLink className="st-job" key={job.slug} href={s.siteHref(`/solutions/${job.slug}`)} data-category={job.category} data-channels={job.channels} hidden={!s.solutionVisible(job.category, job.channels)}>
              <div className="st-job-ticket-wrap">
                <div className="st-job-ticket">
                  <p className="st-job-from">
                    {job.channels.split(",").map((channel) => <ChannelMark key={channel} channel={channel} />)}
                    <span>{job.channels.replaceAll(",", " / ")}</span>
                    {isAvailable(job.slug) && <b className="st-job-ready">{zh ? "可以设置" : "Ready to set up"}</b>}
                  </p>
                  <h2>{job.title}</h2>
                  <p className="st-job-desc">{job.description}</p>
                  <ol className="st-job-lines" aria-label={zh ? "工作流程" : "Workflow"}>
                    {steps.map((step, index) => step && (
                      <li key={step.label}>
                        <WorkflowMarks marks={workflowMarks[job.slug]?.[index] ?? ["check"]} />
                        <span>{step.title}</span>
                      </li>
                    ))}
                  </ol>
                  <div className="st-job-foot">
                    <span className="st-job-boundary"><LockKeyhole size={13} aria-hidden="true" />{job.never}</span>
                    <span className="st-job-go">{zh ? "查看流程" : "See the workflow"}<ArrowRight size={16} aria-hidden="true" /></span>
                  </div>
                </div>
              </div>
            </SiteLink>
          );
        })}
        <SiteLink className="st-job st-custom-job" href={s.siteHref("/nyxbot")}>
          <div className="st-custom-in">
            <Sparkles size={22} aria-hidden="true" />
            <div><h2>{zh ? "还有别的工作？" : "Something else in mind?"}</h2><p>{zh ? "用自己的话描述想完成的事情。" : "Describe what needs doing in your own words."}</p></div>
            <span className="st-job-go">{zh ? "交给 NyxBot" : "Ask NyxBot"}<ArrowRight size={16} aria-hidden="true" /></span>
          </div>
        </SiteLink>
      </div>
    </StorySection>
    {/* Solutions continue to the full list of apps these jobs draw on. */}
    <StoryNext title={zh ? "这些工作用到的应用，以及更多" : "The apps these jobs use, and many more"} description={zh ? "完整目录列出每个可连接的应用及其可用状态。" : "The full list shows every app you can connect and whether it is available."} href={s.siteHref("/integrations")} label={zh ? "工具" : "Tools"} />
  </StoryPage>;
}
