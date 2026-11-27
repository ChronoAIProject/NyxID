import { useState } from "react";
import { Pause, Play } from "lucide-react";
import { usePublicTheme } from "../public-site/public-theme";
import { useWebsite } from "./website-context";
import "./nyxbot-app-media.css";

const mediaRoot = "/landing/nyx/app";
const mobileQuery = "(max-width: 700px)";
const captureSize = {
  task: { desktop: [1200, 640], mobile: [440, 680] },
  memory: { desktop: [563, 570], mobile: [440, 570] },
  approval: { desktop: [601, 161], mobile: [378, 184] },
} as const;
/** Theme and viewport variants are captures of the actual app, with example data. */
function AppStill({ scene, alt, responsive = false, eager = false }: { scene: keyof typeof captureSize; alt: string; responsive?: boolean; eager?: boolean }) {
  const size = captureSize[scene];
  return <div className={`nb-app-still${responsive ? " nb-app-responsive" : ""}`}>
    {(["light", "dark"] as const).map(theme => <picture className={`nb-app-${theme}`} key={theme}>
      {responsive && <source media={mobileQuery} srcSet={`${mediaRoot}/${scene}-${theme}-mobile.webp`} width={size.mobile[0]} height={size.mobile[1]} />}
      <img src={`${mediaRoot}/${scene}-${theme}.webp`} alt={alt} width={size.desktop[0]} height={size.desktop[1]} loading={eager ? "eager" : "lazy"} decoding="async" />
    </picture>)}
  </div>;
}

export function NyxbotAppShowcase() {
  const zh = useWebsite().language === "zh";
  const { theme } = usePublicTheme();
  const [paused, setPaused] = useState(false);
  const extension = paused ? "webp" : "gif";
  const description = zh ? "检查今晚的预订，专属助手把结果回复给 NyxBot。" : "A booking check, delegated to the concierge and reported back to NyxBot.";
  return <div className="st-wrap nb-app-showcase" id="nyxbot-in-action">
    <figure className="nb-app-figure">
      <div className="nb-app-window">
        {theme === null ? <AppStill scene="task" alt={description} responsive eager /> : <picture className="nb-app-gif">
          <source media="(prefers-reduced-motion: reduce) and (max-width: 700px)" srcSet={`${mediaRoot}/task-${theme}-mobile.webp`} width="440" height="680" />
          <source media="(prefers-reduced-motion: reduce)" srcSet={`${mediaRoot}/task-${theme}.webp`} width="1200" height="640" />
          <source media={mobileQuery} srcSet={`${mediaRoot}/task-${theme}-mobile.${extension}`} width="440" height="680" />
          <img src={`${mediaRoot}/task-${theme}.${extension}`} alt={description} width="1200" height="640" decoding="async" />
        </picture>}
      </div>
      <figcaption><span>{zh ? "NyxID 应用，示例数据，英文界面。" : "Captured in the NyxID app with example data."}</span>
        <button className="nb-app-pause" type="button" onClick={() => setPaused(!paused)} aria-label={paused ? zh ? "继续动画" : "Resume animation" : zh ? "暂停动画" : "Pause animation"}>
          {paused ? <Play size={14} aria-hidden="true" /> : <Pause size={14} aria-hidden="true" />}{paused ? zh ? "继续" : "Resume" : zh ? "暂停" : "Pause"}
        </button>
      </figcaption>
    </figure>
  </div>;
}

export function NyxbotMemoryCapture() {
  const zh = useWebsite().language === "zh";
  return <figure className="nb-app-detail nb-app-memory">
    <div className="nb-app-detail-frame"><AppStill scene="memory" responsive alt={zh ? "预订助手的真实记忆面板，保存了早上 8 点汇总预订和无障碍餐桌两条偏好。" : "The reservations specialist’s actual Memory panel, with preferences for an 8:00 booking summary and the wheelchair-accessible table."} /></div>
    <figcaption>{zh ? "专属助手的记忆，示例数据。" : "A specialist’s memory, with example data."}</figcaption>
  </figure>;
}

export function NyxbotApprovalCapture() {
  const zh = useWebsite().language === "zh";
  return <figure className="nb-app-detail nb-app-approval">
    <div className="nb-app-approval-request"><span>{zh ? "你" : "You"}</span><p>{zh ? "取消 Sarah 晚上 7:30 的预订。" : "Cancel Sarah’s 7:30 booking."}</p></div>
    <div className="nb-app-detail-frame"><AppStill scene="approval" responsive alt={zh ? "NyxBot 的真实确认卡：取消 Sarah 晚上 7:30 的预订需要一次性授权，提供 Deny 和 Allow 选项。" : "NyxBot's actual confirmation card asks for one-time permission to cancel Sarah’s 7:30 booking, with Deny and Allow controls."} /></div>
    <figcaption>{zh ? "NyxBot 的确认卡，示例数据。" : "NyxBot’s confirmation card, with example data."}</figcaption>
  </figure>;
}
