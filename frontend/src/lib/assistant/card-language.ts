import type { Language } from "@/i18n/language";
import type { ChatMessage } from "./chat-types";

export type CardLanguageChoice = Language | "auto" | "app";

const englishWords = /\b(?:the|this|that|these|those|please|could|would|should|my|our|with|for|and|from|can|you|your|what|which|how|have|need|want|show|check|send|help)\b/gi;
const traditional = /[這個們請幫檢檔發審閱還讓給為麼處後開關過與問條顯詢報確認應該]/u;
const simplified = /[这个们请帮检档发审阅还让给为么处后开关过与问条显询报确认应该]/u;

type Signal = Language | "zh" | "unsupported";

function messageLanguage(content: string): Signal | undefined {
  const prose = content.slice(0, 4000)
    .replace(/```[\s\S]*?(?:```|$)|`[^`\n]*(?:`|$)|https?:\/\/\S+/g, " ")
    .split("\n")
    .filter(line => !/^\s*(?:>|\{|\[|\$ |\/\/|(?:import|const|let|def|function|SELECT|curl|git|npm|nyxid)\s)/i.test(line))
    .join(" ");
  if (/[\p{Script=Hiragana}\p{Script=Katakana}\p{Script=Hangul}]{2}/u.test(prose)) return "unsupported";
  const han = prose.match(/\p{Script=Han}/gu)?.length ?? 0;
  const english = new Set(prose.toLowerCase().match(englishWords)).size;
  if (han >= 4 && english < 3 && /[的了是我你把用想能要看幫帮請请這这個个將将嗎吗]/u.test(prose)) {
    const hant = traditional.test(prose);
    const hans = simplified.test(prose);
    if (hant && hans) return;
    if (hant) return "zh-TW";
    if (hans) return "zh-CN";
    return "zh";
  }
  if (han) return;
  const words = prose.match(/\p{L}+/gu)?.length ?? 0;
  if (words >= 5 && english >= 2) return "en";
  if (words >= 5 && !english && /[.!?。！？]\s*$/.test(prose)) return "unsupported";
}

/** Require two agreeing human messages. A lone greeting or technical aside cannot switch the UI. */
export function conversationCardLanguage(messages: readonly ChatMessage[], fallback: Language): Language {
  let last: Signal | undefined;
  let neutralChinesePair = false;
  const defaultChinese = fallback === "zh-TW" ? "zh-TW" : "zh-CN";
  for (let index = messages.length - 1; index >= 0; index--) {
    const message = messages[index]!;
    const human = !message.origin || message.origin === "user" || message.origin === "channel";
    if (message.role !== "user" || !human || message.automation || message.continuation_of) continue;
    const language = messageLanguage(message.content);
    if (!language) continue;
    if (last === "zh" && (language === "zh-CN" || language === "zh-TW")) return language;
    if (language === "zh" && (last === "zh-CN" || last === "zh-TW")) return last;
    if (last === "zh" && language === "zh") {
      // These messages establish Chinese. An earlier message may identify its script.
      neutralChinesePair = true;
    } else if (neutralChinesePair) {
      return defaultChinese;
    } else if (last === language && language !== "zh") {
      return language === "unsupported" ? fallback : language;
    }
    last = language;
  }
  return neutralChinesePair ? defaultChinese : fallback;
}
