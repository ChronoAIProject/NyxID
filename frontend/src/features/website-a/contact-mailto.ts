import { createWebsiteContactSchema } from "@/schemas/website-contact";
import type { WebsiteContactData } from "@/schemas/website-contact";

const DEFAULT_CONTACT_EMAIL = "contact@chrono-ai.fun";
const mailbox = /^[a-z0-9._%+-]+@[a-z0-9.-]+\.[a-z]{2,}$/i;

export const contactTopics = [
  ["bundle", "Custom bundle", "定制套餐"],
  ["enterprise", "Enterprise or self-hosting", "企业部署或自托管"],
  ["partner", "Partnerships", "合作"],
  ["support", "Help with my setup", "设置帮助"],
  ["other", "Something else", "其他"],
] as const;

export function resolveContactEmail(configured?: string) {
  if (configured && /[\r\n]/.test(configured)) return DEFAULT_CONTACT_EMAIL;
  const email = configured?.trim();
  return email && mailbox.test(email) ? email : DEFAULT_CONTACT_EMAIL;
}

export function contactEmailHref(email: string) {
  if (!mailbox.test(email) || /[\r\n]/.test(email))
    throw new Error("Invalid contact recipient");
  return `mailto:${encodeURIComponent(email).replace(/%40/g, "@")}`;
}

export function createContactMailto(
  email: string,
  fields: WebsiteContactData,
  language: "en" | "zh",
) {
  const recipient = contactEmailHref(email);
  const data = createWebsiteContactSchema(language).parse(fields);
  const zh = language === "zh";
  const topic = contactTopics.find(([value]) => value === data.topic);
  // Only fixed labels enter the subject. User-provided details stay in body.
  const topicLabel = topic?.[zh ? 2 : 1] ?? (zh ? "联系我们" : "Contact");
  const singleLine = (value: string) => value.replace(/[\r\n]+/g, " ").trim();
  const body = [
    `${zh ? "姓名" : "Name"}: ${singleLine(data.name)}`,
    `${zh ? "回复邮箱" : "Reply email"}: ${singleLine(data.email)}`,
    ...(data.company
      ? [`${zh ? "公司" : "Company"}: ${singleLine(data.company)}`]
      : []),
    `${zh ? "主题" : "Topic"}: ${topicLabel}`,
    "",
    data.message.replace(/\r\n?|\n/g, "\r\n"),
  ].join("\r\n");
  return `${recipient}?subject=${encodeURIComponent(`NyxID — ${topicLabel}`)}&body=${encodeURIComponent(body)}`;
}

export function openContactMailto(href: string) {
  window.location.assign(href);
}
