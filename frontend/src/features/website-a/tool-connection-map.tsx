import { useWebsite } from "./website-context";

const channels = [
  ["telegram", "Telegram"], ["x", "X"], ["whatsapp", "WhatsApp"],
  ["slack", "Slack"], ["discord", "Discord"],
] as const;
const tools = [
  ["google-calendar", "Google Calendar"], ["gmail", "Gmail"],
  ["google-sheets", "Google Sheets"], ["google-drive", "Google Drive"],
  ["notion", "Notion"], ["stripe", "Stripe"],
  ["twilio", "Twilio"], ["github", "GitHub"],
] as const;

/** A compact, native version of the homepage hub; labels and icons stay readable. */
export function ToolConnectionMap() {
  const { language } = useWebsite();
  const zh = language === "zh";
  return (
    <div className="stc-map" role="group" aria-label={zh ? "通过 NyxID 连接聊天渠道与工具" : "Channels connected to tools through NyxID"}>
      <div className="stc-map-col stc-map-in">
        <p className="stc-map-label">{zh ? "从这里发起" : "Ask from"}</p>
        {channels.map(([icon, name]) => <div className="stc-map-tile" key={icon}>
          <img src={`/landing/nyx/icons/${icon}.svg`} width="16" height="16" alt="" />
          <span>{name}{icon === "whatsapp" && <small>{zh ? "即将上线" : "Coming soon"}</small>}</span>
        </div>)}
      </div>
      <svg className="stc-map-wires" viewBox="0 0 100 224" preserveAspectRatio="none" aria-hidden="true">
        {[20, 66, 112, 158, 204].map(y => <path key={y} d={`M0 ${y} C45 ${y} 55 112 100 112`} />)}
      </svg>
      <div className="stc-map-core"><img src="/nyxid-coloured-icon.svg" width="30" height="30" alt="NyxID" /></div>
      <svg className="stc-map-wires" viewBox="0 0 100 224" preserveAspectRatio="none" aria-hidden="true">
        {[31, 85, 139, 193].map(y => <path key={y} d={`M0 112 C45 112 55 ${y} 100 ${y}`} />)}
      </svg>
      <div className="stc-map-col stc-map-out">
        <p className="stc-map-label">{zh ? "在这些工具里执行" : "It works in"}</p>
        {tools.map(([icon, name]) => <div className="stc-map-tile" key={icon}>
          <img src={`/landing/nyx/icons/${icon}.svg`} width="16" height="16" alt="" /><span>{name}</span>
        </div>)}
      </div>
    </div>
  );
}
