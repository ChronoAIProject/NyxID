// Generated from approved A, SHA-256 5ba81989df18ad10986c1f00cebdcb1e2efd294932853a7671685187a7e34d33.
// Run: node scripts/website-a/generate.mjs <baseline.html>
import { CodeExample } from "../../interactions";
import { VisualLink, RevealFrame } from "../../visuals/elements";
import { useWebsite } from "../../website-context";
export default function AgentsPage() {
  const s = useWebsite();
  void s;
  return (
    <>
      {"\n"}
      <section className="hero glowbg" style={{ position: "relative" }}>
        {"\n  "}
        <div className="wrap">
          <div className="over v">{"接入你的智能体"}</div>
          <h1
            className="h1"
            style={{ marginTop: "14px", fontSize: "var(--fs-h1)" }}
          >
            {"已经在用 AI 智能体？"}
            <br />
            <span className="grad">{"把它接入 NyxID。"}</span>
          </h1>
          <p className="lead">
            {
              "Claude、ChatGPT、Cursor、Codex 或你自己的脚本：把它指向 NyxID，就能获得对你的服务、AI 模型和机器的限权访问，而且永远不用持有密码。NyxID 是开源的：可自行部署，也可使用我们的托管服务。"
            }
          </p>
          <div className="cta">
            <VisualLink
              className="btn p lg"
              href={s.authHref("register")}
              effect="button"
            >
              {"获取智能体密钥"}
            </VisualLink>
            <a
              className="btn lg"
              href={s.siteHref("/docs/ai/guides/agent-isolation")}
            >
              {"阅读：限权密钥"}
            </a>
          </div>
        </div>
        {"\n"}
      </section>
      {"\n"}
      <section className="s">
        <div className="wrap">
          <div className="grid g4">
            {"\n  "}
            <div className="card">
              <div className="ic">
                <svg className="i" viewBox="0 0 24 24">
                  <polyline points="16 18 22 12 16 6"></polyline>
                  <polyline points="8 6 2 12 8 18"></polyline>
                </svg>
              </div>
              <h3 style={{ fontSize: "var(--fs-h3)", margin: "14px 0 6px" }}>
                {"REST 代理"}
              </h3>
              <p className="small" style={{ fontSize: "14px" }}>
                {"/api/v1/proxy/s/{service}/…：NyxID 注入凭据并执行权限范围。"}
              </p>
            </div>
            <div className="card">
              <div className="ic">
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M12 3l1.9 5.8L20 11l-6.1 2.2L12 19l-1.9-5.8L4 11l6.1-2.2z"></path>
                </svg>
              </div>
              <h3 style={{ fontSize: "var(--fs-h3)", margin: "14px 0 6px" }}>
                {"LLM 网关"}
              </h3>
              <p className="small" style={{ fontSize: "14px" }}>
                {"兼容 OpenAI 的端点，接入你连接的所有模型。"}
              </p>
            </div>
            <div className="card">
              <div className="ic">
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M12 22v-5M9 8V2M15 8V2M18 8v5a6 6 0 0 1-12 0V8z"></path>
                </svg>
              </div>
              <h3 style={{ fontSize: "var(--fs-h3)", margin: "14px 0 6px" }}>
                {"MCP 服务器"}
              </h3>
              <p className="small" style={{ fontSize: "14px" }}>
                {"连接 Claude、Cursor、Codex 等，服务以工具形式呈现。"}
              </p>
            </div>
            <div className="card">
              <div className="ic">
                <svg className="i" viewBox="0 0 24 24">
                  <rect x="2" y="3" width="20" height="14" rx="2"></rect>
                  <path d="M8 21h8M12 17v4"></path>
                </svg>
              </div>
              <h3 style={{ fontSize: "var(--fs-h3)", margin: "14px 0 6px" }}>
                {"机器节点"}
              </h3>
              <p className="small" style={{ fontSize: "14px" }}>
                {
                  "nyxid node setup --machine：在你允许的范围内让智能体使用浏览器、文件或命令行。"
                }
              </p>
            </div>
            {"\n"}
          </div>
        </div>
      </section>
      {"\n"}
      <section className="s">
        <div className="wrap split">
          {"\n  "}
          <div>
            <h2 className="h2">{"你的智能体获得什么"}</h2>
            <ul className="checks" style={{ marginTop: "24px" }}>
              <li>
                <span className="ck">
                  <svg className="i" viewBox="0 0 24 24">
                    <circle cx="7.5" cy="15.5" r="5.5"></circle>
                    <path d="m21 2-9.6 9.6M15.5 7.5l3 3L22 7l-3-3"></path>
                  </svg>
                </span>
                <div>
                  <b>{"Agent Keys"}</b>
                  <span>
                    {"每个智能体一个 nyxid_ag_ 密钥，带服务和节点白名单。"}
                  </span>
                </div>
              </li>
              <li>
                <span className="ck">
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z"></path>
                  </svg>
                </span>
                <div>
                  <b>{"操作范围"}</b>
                  <span>
                    {"允许 create_event、禁止 delete_event，还可限制输入。"}
                  </span>
                </div>
              </li>
              <li>
                <span className="ck">
                  <svg className="i" viewBox="0 0 24 24">
                    <circle cx="12" cy="12" r="10"></circle>
                    <path d="M12 6v6l4 2"></path>
                  </svg>
                </span>
                <div>
                  <b>{"按智能体限流"}</b>
                  <span>{"每个密钥独立计数；响应中带 X-NyxID-Agent-Id。"}</span>
                </div>
              </li>
              <li>
                <span className="ck">
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M8 6h13M8 12h13M8 18h13M3 6h.01M3 12h.01M3 18h.01"></path>
                  </svg>
                </span>
                <div>
                  <b>{"可归属的审计"}</b>
                  <span>{"每次调用都记录发起它的密钥。"}</span>
                </div>
              </li>
            </ul>
          </div>
          {"\n  "}
          <CodeExample
            labels={["curl", "LLM", "MCP", "CLI"]}
            snippets={[
              <>
                <span className="c">
                  {
                    "# 你的智能体通过 NyxID 订桌。\n# NyxID 注入凭据、检查权限范围并记录调用。"
                  }
                </span>
                {"\ncurl -X POST \\\n  "}
                <span className="s">
                  {
                    "https://nyx-api.chrono-ai.fun/api/v1/proxy/s/google-calendar/calendars/primary/events"
                  }
                </span>
                {" \\\n  -H "}
                <span className="s">
                  {'"Authorization: Bearer $NYXID_AGENT_KEY"'}
                </span>
                {" \\\n  -d "}
                <span className="s">
                  {
                    '\'{"summary":"Sarah · table for 4","start":{"dateTime":"2026-10-18T19:30:00"}}\''
                  }
                </span>
                {"\n\n"}
                <span className="g">{"→ 200 OK"}</span>
                {"   "}
                <span className="c">{"# 允许：calendar create_event"}</span>
                {"\n"}
                <span className="k">{"→ 403"}</span>
                {"      "}
                <span className="c">{"# 同一密钥尝试 delete_event 时"}</span>
              </>,
              <>
                <span className="c">
                  {"# 一个兼容 OpenAI 的端点，接入你连接的所有模型"}
                </span>
                {"\ncurl "}
                <span className="s">
                  {
                    "https://nyx-api.chrono-ai.fun/api/v1/llm/gateway/v1/chat/completions"
                  }
                </span>
                {" \\\n  -H "}
                <span className="s">
                  {'"Authorization: Bearer $NYXID_AGENT_KEY"'}
                </span>
                {" \\\n  -d "}
                <span className="s">
                  {
                    '\'{"model":"claude-sonnet-5-5","messages":[{"role":"user","content":"Summarise today’s bookings"}]}\''
                  }
                </span>
              </>,
              <>
                <span className="c">
                  {"// Claude、Cursor、Codex……任何 MCP 客户端都能连接 NyxID"}
                </span>
                {"\n{\n  "}
                <span className="s">{'"mcpServers"'}</span>
                {": {\n    "}
                <span className="s">{'"nyxid"'}</span>
                {": { "}
                <span className="s">{'"url"'}</span>
                {": "}
                <span className="s">
                  {'"https://nyx-api.chrono-ai.fun/mcp"'}
                </span>
                {" }\n  }\n}\n"}
                <span className="c">
                  {"// 你连接的服务会以工具形式出现，并受此密钥的权限限制。"}
                </span>
              </>,
              <>
                <span className="c">{"# 为每个智能体分配独立的受限身份"}</span>
                {"\nnyxid api-key create --name "}
                <span className="s">{'"reservations-bot"'}</span>
                {
                  " --platform claude-code\nnyxid api-key bind reservations-bot --service google-calendar --credential "
                }
                <span className="s">{'"Luna Bistro"'}</span>
                {"\n\n"}
                <span className="c">{"# 连接聊天渠道"}</span>
                {
                  "\nnyxid channel-bot register --platform telegram --label luna --token-env TELEGRAM_BOT_TOKEN\n\n"
                }
                <span className="c">
                  {
                    "# 让 NyxBot 使用这台电脑（浏览器/文件/命令行默认关闭，需你开启）"
                  }
                </span>
                {"\nnyxid node setup --machine --computer"}
              </>,
            ]}
          />
          {"\n"}
        </div>
      </section>
      {"\n"}
      <section className="s">
        <div className="wrap">
          <h2 className="h2" style={{ marginBottom: "28px" }}>
            {"资源"}
          </h2>
          <RevealFrame className="grid g4">
            <VisualLink
              className="card clk"
              href="https://github.com/ChronoAIProject/NyxID"
              target="_blank"
              rel="noopener noreferrer"
              effect="card"
            >
              <b>{"GitHub 源码"}</b>
              <p className="small" style={{ marginTop: "6px" }}>
                {"Apache-2.0 · 用 Docker 自托管"}
              </p>
              <span className="more" style={{ marginTop: "12px" }}>
                {"打开 "}
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M5 12h14M12 5l7 7-7 7"></path>
                </svg>
              </span>
            </VisualLink>
            <VisualLink
              className="card clk"
              href={s.siteHref("/docs/ai/guides/mcp-proxy")}
              effect="card"
            >
              <b>{"MCP 设置指南"}</b>
              <p className="small" style={{ marginTop: "6px" }}>
                {"阅读文档"}
              </p>
              <span className="more" style={{ marginTop: "12px" }}>
                {"打开 "}
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M5 12h14M12 5l7 7-7 7"></path>
                </svg>
              </span>
            </VisualLink>
            <VisualLink
              className="card clk"
              href={s.siteHref("/docs/ai/guides/agent-isolation")}
              effect="card"
            >
              <b>{"智能体隔离"}</b>
              <p className="small" style={{ marginTop: "6px" }}>
                {"阅读文档"}
              </p>
              <span className="more" style={{ marginTop: "12px" }}>
                {"打开 "}
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M5 12h14M12 5l7 7-7 7"></path>
                </svg>
              </span>
            </VisualLink>
            <VisualLink
              className="card clk"
              href={s.siteHref("/docs/shared/concepts/credential-nodes")}
              effect="card"
            >
              <b>{"机器节点"}</b>
              <p className="small" style={{ marginTop: "6px" }}>
                {"阅读文档"}
              </p>
              <span className="more" style={{ marginTop: "12px" }}>
                {"打开 "}
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M5 12h14M12 5l7 7-7 7"></path>
                </svg>
              </span>
            </VisualLink>
          </RevealFrame>
        </div>
      </section>
      {"\n"}
      <section className="s">
        <div className="wrap">
          <div className="band">
            <h2 className="h2">{"给智能体权限，而不是你的密码。"}</h2>
            <p style={{ opacity: ".88", marginTop: "12px", fontSize: "17px" }}>
              {"平台原价、零加价。开源。从第一天起就使用限权密钥。"}
            </p>
            <div
              style={{
                display: "flex",
                gap: "12px",
                justifyContent: "center",
                flexWrap: "wrap",
                marginTop: "28px",
                position: "relative",
                zIndex: "1",
              }}
            >
              <a className="btn lg" href={s.authHref("register")}>
                {"开始使用"}
              </a>
              <a className="btn lg gh" href={s.siteHref("/solutions")}>
                {"浏览方案"}
              </a>
            </div>
          </div>
        </div>
      </section>
    </>
  );
}
