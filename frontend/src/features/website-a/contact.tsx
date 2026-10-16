import { Fragment, useEffect, useMemo, useRef, useState } from "react";
import type { ReactNode } from "react";
import type { UseFormReturn } from "react-hook-form";
import { useLocation } from "@tanstack/react-router";
import { zodResolver } from "@hookform/resolvers/zod";
import { Send, Sparkles } from "lucide-react";
import { useAppForm } from "@/components/ui/form";
import {
  contactTopicFromSearch,
  createWebsiteContactSchema,
  createWebsiteQuestionSchema,
} from "@/schemas/website-contact";
import type { WebsiteContactData } from "@/schemas/website-contact";
import { useWebsite } from "./website-context";
import "./contact.css";

const topics = [
  ["bundle", "Custom bundle", "定制套餐"],
  ["enterprise", "Enterprise or self-hosting", "企业部署或自托管"],
  ["partner", "Partnerships", "合作"],
  ["support", "Help with my setup", "设置帮助"],
  ["other", "Something else", "其他"],
] as const;
const questions = [
  ["How does pricing work?", "怎么收费？"],
  ["I need a custom bundle", "我需要定制套餐"],
  ["Can I self-host?", "可以自托管吗？"],
  ["Is it secure?", "安全吗？"],
] as const;

type Reply = "bundle" | "pricing" | "selfhost" | "security" | "other";
function replyFor(question: string): Reply {
  const text = question.toLowerCase();
  if (/bundle|enterprise|team|volume|套餐|企业|团队/.test(text))
    return "bundle";
  if (/price|pricing|cost|markup|fee|pay|收费|价格|费用|多少钱/.test(text))
    return "pricing";
  if (/self|host|docker|open|source|github|自托管|开源|部署/.test(text))
    return "selfhost";
  if (/secur|safe|password|key|vault|安全|密码|密钥/.test(text))
    return "security";
  return "other";
}

function ContactForm({ form }: { form: UseFormReturn<WebsiteContactData> }) {
  const { language } = useWebsite();
  const zh = language === "zh";
  const [checked, setChecked] = useState(false);
  const errors = form.formState.errors;
  const error = (name: keyof WebsiteContactData) =>
    errors[name] ? (
      <p className="auth-err" id={`${name}-error`} role="alert">
        {errors[name]?.message}
      </p>
    ) : null;
  const field = (name: keyof WebsiteContactData) => ({
    "aria-invalid": !!errors[name],
    "aria-describedby": errors[name] ? `${name}-error` : undefined,
  });
  return (
    <div className="card contact-form">
      <div className="over">{zh ? "发邮件" : "Email us"}</div>
      <h2
        className="h3"
        style={{ fontSize: "var(--fs-h3)", margin: "6px 0 4px" }}
      >
        {zh ? "给我们留言" : "Drop us a note"}
      </h2>
      <p className="small" id="contact-delivery-note">
        {zh
          ? "发送渠道接入中。当前填写内容不会发送或保存，请通过右侧 Discord 联系团队。"
          : "Email delivery is being connected. Entries are not sent or saved; contact the team through Discord for now."}
      </p>
      <form
        id="cForm"
        noValidate
        aria-describedby="contact-delivery-note"
        onSubmit={form.handleSubmit(() => setChecked(true))}
      >
        <div className="f2">
          <div>
            <label className="auth-label" htmlFor="cName">
              {zh ? "姓名" : "Name"}
            </label>
            <input
              id="cName"
              className="c-input"
              autoComplete="name"
              required
              {...form.register("name")}
              {...field("name")}
            />
            {error("name")}
          </div>
          <div>
            <label className="auth-label" htmlFor="cEmail">
              {zh ? "工作邮箱" : "Work email"}
            </label>
            <input
              id="cEmail"
              className="c-input"
              type="email"
              autoComplete="email"
              required
              {...form.register("email")}
              {...field("email")}
            />
            {error("email")}
          </div>
        </div>
        <div className="f2">
          <div>
            <label className="auth-label" htmlFor="cCo">
              {zh ? "公司" : "Company"}{" "}
              <span className="small">{zh ? "（选填）" : "(optional)"}</span>
            </label>
            <input
              id="cCo"
              className="c-input"
              autoComplete="organization"
              {...form.register("company")}
            />
            {error("company")}
          </div>
          <div>
            <label className="auth-label" htmlFor="cTopic">
              {zh ? "主题" : "Topic"}
            </label>
            <select
              id="cTopic"
              className="c-input"
              {...form.register("topic")}
              {...field("topic")}
            >
              <option value="">{zh ? "选择主题" : "Choose a topic"}</option>
              {topics.map(([value, en, cn]) => (
                <option key={value} value={value}>
                  {zh ? cn : en}
                </option>
              ))}
            </select>
            {error("topic")}
          </div>
        </div>
        <div>
          <label className="auth-label" htmlFor="cMsg">
            {zh ? "留言" : "Message"}
          </label>
          <textarea
            id="cMsg"
            className="c-input"
            rows={5}
            required
            {...form.register("message")}
            {...field("message")}
          />
          {error("message")}
        </div>
        <button className="btn p lg" type="submit" style={{ width: "100%" }}>
          {zh ? "发送" : "Send message"}
        </button>
        {checked && (
          <p className="small" role="status">
            {zh
              ? "尚未发送：发送渠道未接通，填写内容仍留在此页。"
              : "Not sent: email delivery is not connected. Your entries remain on this page."}
          </p>
        )}
      </form>
    </div>
  );
}

function ContactDemo({ selectBundle }: { selectBundle: () => void }) {
  const s = useWebsite();
  const zh = s.language === "zh";
  const schema = useMemo(
    () => createWebsiteQuestionSchema(s.language),
    [s.language],
  );
  const form = useAppForm({
    resolver: zodResolver(schema),
    defaultValues: { question: "" },
  });
  const [messages, setMessages] = useState<
    { question: string; reply: Reply }[]
  >([]);
  const log = useRef<HTMLDivElement>(null);
  useEffect(() => {
    if (log.current) log.current.scrollTop = log.current.scrollHeight;
  }, [messages]);
  function ask(question: string) {
    const parsed = schema.safeParse({ question });
    if (!parsed.success) return;
    const reply = replyFor(parsed.data.question);
    setMessages((previous) => [
      ...previous.slice(-15),
      { question: parsed.data.question, reply },
    ]);
    if (reply === "bundle") selectBundle();
    form.reset({ question: "" });
  }
  function answer(reply: Reply) {
    switch (reply) {
      case "bundle":
        return zh
          ? "套餐可围绕用量、可预期支出、SSO、部署和支持定制。左侧已选择“定制套餐”；当前表单不会发送信息，可先到 Discord 与团队交流。"
          : "Bundles can cover volume, predictable spend, SSO, deployment and support. Custom bundle is selected in the form; it does not send yet, so talk to the team on Discord for now.";
      case "pricing":
        return (
          <>
            {zh
              ? "按服务商公布的价格付费，NyxID 不加价。详见"
              : "You pay each provider’s published rate with no NyxID markup. See the "}
            <a href={s.siteHref("/pricing")}>
              {zh ? "价格页" : "pricing page"}
            </a>
            {zh ? "。" : "."}
          </>
        );
      case "selfhost":
        return (
          <>
            {zh
              ? "可以。NyxID 基于 Apache-2.0 开源，可用 Docker 部署。查看"
              : "Yes. NyxID is open source under Apache-2.0 and runs with Docker. See the "}
            <a href={s.siteHref("/docs")}>
              {zh ? "部署文档" : "documentation"}
            </a>
            {zh ? "。" : "."}
          </>
        );
      case "security":
        return (
          <>
            {zh
              ? "登录信息保存在加密保险库中，智能体使用限权密钥，高风险操作需要确认。详见"
              : "Logins stay in an encrypted vault; agents use scoped keys and risky actions require approval. See the "}
            <a href={s.siteHref("/security")}>
              {zh ? "安全页" : "security page"}
            </a>
            {zh ? "。" : "."}
          </>
        );
      default:
        return zh
          ? "这是本地脚本演示，只回答这些常见问题。其他问题请通过下方 Discord 联系团队；使用真实 NyxBot 请先登录。"
          : "This local scripted demo only covers these common questions. Ask the team on Discord for anything else; sign in to use the real NyxBot.";
    }
  }
  return (
    <div className="card bot">
      <div className="bot-h">
        <span className="nb-av sm">
          <Sparkles size={16} aria-hidden="true" />
        </span>
        <div>
          <b style={{ fontSize: 14 }}>
            {zh ? "和 NyxBot 聊聊" : "Chat with NyxBot"}
          </b>
          <div className="small" style={{ fontSize: 12 }}>
            {zh ? "常见问题，即时回答" : "Instant answers to common questions"}
          </div>
        </div>
      </div>
      <p className="small" id="contact-demo-note">
        {zh
          ? "预设问答演示：回复来自本地脚本，不连接 AI 服务。"
          : "Scripted FAQ demo: replies are local and do not connect to an AI service."}{" "}
        <a href={s.authHref("login")}>
          {zh ? "登录使用 NyxBot" : "Sign in to use NyxBot"}
        </a>
      </p>
      <div
        ref={log}
        className="msgs bot-msgs"
        id="botMsgs"
        role="log"
        aria-label={zh ? "NyxBot 演示问答" : "NyxBot demo conversation"}
      >
        <div className="m out">
          {zh
            ? "你好！可以问我价格、套餐、自托管或安全方面的问题。"
            : "Hi! Ask me about pricing, bundles, self-hosting or security."}
        </div>
        {messages.map((message, index) => (
          <Fragment key={index}>
            <div className="m in">{message.question}</div>
            <div className="m out">{answer(message.reply)}</div>
          </Fragment>
        ))}
      </div>
      <div className="pills bot-chips" id="botChips">
        {questions.map(([en, cn]) => (
          <button
            className="pill"
            type="button"
            key={en}
            onClick={() => ask(zh ? cn : en)}
          >
            {zh ? cn : en}
          </button>
        ))}
      </div>
      <form
        className="bot-in"
        id="botForm"
        noValidate
        aria-describedby="contact-demo-note"
        onSubmit={form.handleSubmit(({ question }) => ask(question))}
      >
        <input
          id="botText"
          className="c-input"
          maxLength={1000}
          placeholder={zh ? "输入问题……" : "Type a question…"}
          aria-label={zh ? "给 NyxBot 发消息" : "Message NyxBot"}
          aria-invalid={!!form.formState.errors.question}
          aria-describedby={
            form.formState.errors.question ? "bot-question-error" : undefined
          }
          {...form.register("question")}
        />
        <button
          className="btn"
          type="submit"
          aria-label={zh ? "发送问题" : "Send question"}
        >
          <Send size={16} aria-hidden="true" />
        </button>
      </form>
      {form.formState.errors.question && (
        <p className="auth-err" id="bot-question-error" role="alert">
          {form.formState.errors.question.message}
        </p>
      )}
    </div>
  );
}

export function ContactSection({ children }: { children: ReactNode }) {
  const { language } = useWebsite();
  const location = useLocation();
  const topic = contactTopicFromSearch(location.searchStr);
  const schema = useMemo(
    () => createWebsiteContactSchema(language),
    [language],
  );
  const form = useAppForm<WebsiteContactData>({
    resolver: zodResolver(schema),
    defaultValues: { name: "", email: "", company: "", topic, message: "" },
  });
  const { setValue } = form;
  useEffect(() => {
    setValue("topic", topic, { shouldDirty: false, shouldTouch: false });
  }, [topic, setValue]);
  return (
    <div className="wrap contact">
      <ContactForm form={form} />
      <div className="contact-side">
        <ContactDemo selectBundle={() => setValue("topic", "bundle")} />
        {children}
      </div>
    </div>
  );
}
