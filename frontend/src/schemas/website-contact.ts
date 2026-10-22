import { z } from "zod";

export const websiteContactTopicSchema = z.enum([
  "",
  "bundle",
  "enterprise",
  "partner",
  "support",
  "other",
]);

export function contactTopicFromSearch(search: string) {
  const topic = websiteContactTopicSchema.safeParse(
    new URLSearchParams(search).get("topic") ?? "",
  );
  return topic.success ? topic.data : "";
}

export function createWebsiteContactSchema(language: "en" | "zh") {
  const zh = language === "zh";
  return z.object({
    name: z
      .string()
      .trim()
      .min(1, zh ? "请填写姓名。" : "Add your name."),
    email: z
      .string()
      .trim()
      .regex(
        /^[^\s@]+@[^\s@]+\.[^\s@]+$/,
        zh ? "请填写有效邮箱。" : "Add a valid email.",
      ),
    company: z.string().trim(),
    topic: websiteContactTopicSchema,
    message: z
      .string()
      .trim()
      .min(1, zh ? "请填写简短留言。" : "Add a short message."),
  });
}

export function createWebsiteQuestionSchema(language: "en" | "zh") {
  return z.object({
    question: z
      .string()
      .trim()
      .min(1, language === "zh" ? "请输入问题。" : "Enter a question.")
      .max(
        1000,
        language === "zh"
          ? "问题请控制在 1000 字以内。"
          : "Keep your question under 1,000 characters.",
      ),
  });
}

export type WebsiteContactData = z.infer<
  ReturnType<typeof createWebsiteContactSchema>
>;
