import { AlignLeft, Bell, Check, Clock, FileCheck2, Globe, ListFilter, MessageCircle, Monitor, PackageCheck, Phone, Search } from "lucide-react";

const glyphs = { summary: AlignLeft, bell: Bell, check: Check, clock: Clock, file: FileCheck2, browser: Globe, filter: ListFilter, message: MessageCircle, computer: Monitor, package: PackageCheck, phone: Phone, search: Search };
const brands = ["telegram", "x", "google-calendar", "google-sheets", "google-docs", "twilio", "elevenlabs"] as const;
type Brand = (typeof brands)[number];
type Mark = Brand | keyof typeof glyphs;

// Each array describes trigger / action / result. This is deliberately authored
// independently of prose, so translation or mentioning another app cannot
// silently attach that app's logo to the wrong action.
export const workflowMarks: Record<string, readonly (readonly Mark[])[]> = {
  "restaurant-manager": [["telegram"], ["google-calendar", "google-sheets"], ["telegram"]],
  "help-desk": [["telegram", "x"], ["message"], ["check"]],
  "appointment-booker": [["telegram"], ["google-calendar"], ["bell"]],
  "callback-queue": [["google-sheets"], ["twilio", "elevenlabs"], ["google-sheets"]],
  "stock-analyst": [["telegram", "clock"], ["summary"], ["telegram", "google-sheets"]],
  "news-aggregator": [["clock"], ["filter"], ["telegram"]],
  "desk-researcher": [["telegram"], ["search"], ["google-docs"]],
  "x-lead-catcher": [["x"], ["google-sheets"], ["google-sheets"]],
  "back-office-clerk": [["telegram", "clock"], ["browser"], ["package"]],
};

export function WorkflowMarks({ marks }: { marks: readonly Mark[] }) {
  return <span className="st-job-mark" aria-hidden="true">{marks.map(mark => {
    if (brands.includes(mark as Brand)) return <img key={mark} src={`/landing/nyx/icons/${mark}.svg`} width="18" height="18" alt="" />;
    const Glyph = glyphs[mark as keyof typeof glyphs];
    return <Glyph key={mark} size={16} />;
  })}</span>;
}

export function ChannelMark({ channel }: { channel: string }) {
  if (channel === "Phone") return <Phone size={15} aria-hidden="true" />;
  if (channel === "Computer") return <Monitor size={15} aria-hidden="true" />;
  if (channel === "Telegram" || channel === "X") return <img src={`/landing/nyx/icons/${channel.toLowerCase()}.svg`} width="16" height="16" alt="" />;
  return null;
}
