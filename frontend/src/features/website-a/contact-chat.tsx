import { useState } from "react";
import { zodResolver } from "@hookform/resolvers/zod";
import { useAppForm } from "@/components/ui/form";
import { createWebsiteQuestionSchema } from "@/schemas/website-contact";
import { websiteRelease } from "./release-config";
import type { ChatConfig } from "./release-config";
import { useWebsite } from "./website-context";
import { askPublicNyxBot } from "./public-feature-adapters";
import { usePublicFeatureRequest } from "./use-public-feature-request";
export function ContactChat({
  config = websiteRelease.contactChat,
}: {
  config?: ChatConfig | null;
}) {
  if (!config) return null;
  return <ChatCard config={config} />;
}
function ChatCard({ config }: { config: ChatConfig }) {
  const { language } = useWebsite(),
    zh = language === "zh";
  const form = useAppForm<{ question: string }>({
    resolver: zodResolver(createWebsiteQuestionSchema(language)),
    defaultValues: { question: "" },
  });
  const request = usePublicFeatureRequest();
  const [exchange, setExchange] = useState<{
    question: string;
    reply: string;
  } | null>(null);
  async function submit({ question }: { question: string }) {
    const result = await request.run((signal) =>
      askPublicNyxBot(config, question, language, signal),
    );
    if (result) {
      setExchange({ question, reply: result.reply });
      form.reset();
    }
  }
  const suggestions = zh
    ? ["套餐有哪些？", "可以自托管吗？", "如何保护凭证？"]
    : [
        "What bundles are available?",
        "Can I self-host?",
        "How are credentials protected?",
      ];
  return (
    <div className="card bot" data-website-feature="contact-chat">
      <div className="bot-h">
        <div>
          <b>{zh ? "和 NyxBot 聊聊" : "Chat with NyxBot"}</b>
          <div className="small">
            {zh
              ? "请勿输入密码、密钥或敏感信息。"
              : "Do not enter passwords, keys or sensitive information."}
          </div>
        </div>
      </div>
      <div
        className="msgs bot-msgs"
        role="log"
        aria-label={zh ? "公共 NyxBot 回答" : "Public NyxBot answers"}
      >
        {exchange && (
          <>
            <div className="m in">{exchange.question}</div>
            <div className="m out">{exchange.reply}</div>
          </>
        )}
      </div>
      <div className="pills bot-chips">
        {suggestions.map((text) => (
          <button
            className="pill"
            type="button"
            key={text}
            disabled={!request.interactive || request.pending}
            onClick={() => form.setValue("question", text)}
          >
            {text}
          </button>
        ))}
      </div>
      <form className="bot-in" noValidate onSubmit={form.handleSubmit(submit)}>
        <fieldset
          disabled={!request.interactive || request.pending}
          style={{ display: "contents" }}
        >
          <input
            className="c-input"
            aria-label={zh ? "给 NyxBot 发消息" : "Message NyxBot"}
            maxLength={1000}
            required
            {...form.register("question")}
          />
          <button
            className="btn"
            type="submit"
            disabled={!request.interactive || request.pending}
          >
            {zh ? "发送" : "Send"}
          </button>
        </fieldset>
      </form>
      {form.formState.errors.question && (
        <p role="alert" className="small">
          {zh
            ? "请输入 1–1000 字的问题。"
            : "Enter a question of 1–1,000 characters."}
        </p>
      )}
      {request.failed && (
        <p role="alert" className="small">
          {zh
            ? "暂时无法回答，请重试或使用下方邮件、Discord 联系方式。"
            : "Answers are temporarily unavailable. Retry or use email or Discord."}
        </p>
      )}
    </div>
  );
}
