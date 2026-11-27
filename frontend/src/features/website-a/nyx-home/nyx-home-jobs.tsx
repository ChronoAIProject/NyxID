import { useWebsite } from "../website-context";
import copy from "../solution-story-copy.json";
import { workflowMarks, WorkflowMarks, ChannelMark } from "../solution-workflow-icons";

const jobs = ["restaurant-manager", "news-aggregator", "callback-queue", "x-lead-catcher", "desk-researcher", "back-office-clerk"];

export function HomeJobs() {
  const s = useWebsite();
  return <div className="jobs-rail" data-jobs-rail=""><div className="jobs-track" data-jobs-track="">
    {jobs.map(slug => {
      const job = copy[s.language].find(job => job.slug === slug)!;
      return <a className="job" href={s.siteHref(`/solutions/${slug}`)} key={slug}>
        <div className="ticket-wrap"><div className="ticket">
          <p className="job-from">{job.channels.split(",").map(channel => <ChannelMark key={channel} channel={channel} />)}{job.channels.replaceAll(",", " / ")}</p>
          <h3>{job.title}</h3><p className="job-desc">{job.description}</p>
          <ol className="t-lines">{[job.flow[0], job.flow[2], job.flow[4]].map((step, index) => <li className="t-line" key={step!.label}><WorkflowMarks marks={workflowMarks[slug]![index]!} /><p className="t-what">{step!.title}</p></li>)}</ol>
          <p className="job-go">{s.language === "zh" ? "查看流程" : "See the workflow"}<span className="arr" aria-hidden="true">→</span></p>
        </div></div>
      </a>;
    })}
  </div></div>;
}
