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
          <div className="over v">{"Connect your agent"}</div>
          <h1
            className="h1"
            style={{ marginTop: "14px", fontSize: "var(--fs-h1)" }}
          >
            {"Already use an AI agent?"}
            <br />
            <span className="grad">{"Plug it into NyxID."}</span>
          </h1>
          <p className="lead">
            {
              "Claude, ChatGPT, Cursor, Codex or your own script: point it at NyxID and it gets scoped access to your services, AI models and machines, without ever holding a password. NyxID is open source: self-host it, or use ours."
            }
          </p>
          <div className="cta">
            <VisualLink
              className="btn p lg"
              href={s.authHref("register")}
              effect="button"
            >
              {"Get an agent key"}
            </VisualLink>
            <a
              className="btn lg"
              href={s.siteHref("/docs/ai/guides/agent-isolation")}
            >
              {"Read: scoped keys"}
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
                {"REST proxy"}
              </h3>
              <p className="small" style={{ fontSize: "14px" }}>
                {
                  "/api/v1/proxy/s/{service}/…: NyxID injects credentials and enforces scope."
                }
              </p>
            </div>
            <div className="card">
              <div className="ic">
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M12 3l1.9 5.8L20 11l-6.1 2.2L12 19l-1.9-5.8L4 11l6.1-2.2z"></path>
                </svg>
              </div>
              <h3 style={{ fontSize: "var(--fs-h3)", margin: "14px 0 6px" }}>
                {"LLM gateway"}
              </h3>
              <p className="small" style={{ fontSize: "14px" }}>
                {"OpenAI-compatible endpoint for every model you connect."}
              </p>
            </div>
            <div className="card">
              <div className="ic">
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M12 22v-5M9 8V2M15 8V2M18 8v5a6 6 0 0 1-12 0V8z"></path>
                </svg>
              </div>
              <h3 style={{ fontSize: "var(--fs-h3)", margin: "14px 0 6px" }}>
                {"MCP server"}
              </h3>
              <p className="small" style={{ fontSize: "14px" }}>
                {
                  "Connect Claude, Cursor, Codex and others; services appear as tools."
                }
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
                {"Machine nodes"}
              </h3>
              <p className="small" style={{ fontSize: "14px" }}>
                {
                  "nyxid node setup --machine: let agents use a browser, files or shell where you allow."
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
            <h2 className="h2">{"What your agent gets"}</h2>
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
                    {
                      "One nyxid_ag_ key per agent, with service and node allowlists."
                    }
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
                  <b>{"Operation scopes"}</b>
                  <span>
                    {
                      "Allow create_event and deny delete_event, with optional input limits."
                    }
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
                  <b>{"Per-agent rate limits"}</b>
                  <span>
                    {"Separate buckets per key; X-NyxID-Agent-Id on responses."}
                  </span>
                </div>
              </li>
              <li>
                <span className="ck">
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M8 6h13M8 12h13M8 18h13M3 6h.01M3 12h.01M3 18h.01"></path>
                  </svg>
                </span>
                <div>
                  <b>{"Attributed audit"}</b>
                  <span>
                    {"Every call is logged with the key that made it."}
                  </span>
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
                    "# Your agent books a table through NyxID.\n# NyxID injects the credential, checks the scope, logs the call."
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
                <span className="c">{"# allowed: calendar create_event"}</span>
                {"\n"}
                <span className="k">{"→ 403"}</span>
                {"      "}
                <span className="c">
                  {"# if the same key tries delete_event"}
                </span>
              </>,
              <>
                <span className="c">
                  {
                    "# One OpenAI-compatible endpoint for every model you connect"
                  }
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
                  {"// Claude, Cursor, Codex… connect any MCP client to NyxID"}
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
                  {
                    "// Your connected services appear as tools, scoped to this key."
                  }
                </span>
              </>,
              <>
                <span className="c">
                  {"# Give each agent its own limited identity"}
                </span>
                {"\nnyxid api-key create --name "}
                <span className="s">{'"reservations-bot"'}</span>
                {
                  " --platform claude-code\nnyxid api-key bind reservations-bot --service google-calendar --credential "
                }
                <span className="s">{'"Luna Bistro"'}</span>
                {"\n\n"}
                <span className="c">{"# Connect a chat channel"}</span>
                {
                  "\nnyxid channel-bot register --platform telegram --label luna --token-env TELEGRAM_BOT_TOKEN\n\n"
                }
                <span className="c">
                  {
                    "# Let NyxBot use this computer (browser/files/shell off until you enable them)"
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
            {"Resources"}
          </h2>
          <RevealFrame className="grid g4">
            <VisualLink
              className="card clk"
              href="https://github.com/ChronoAIProject/NyxID"
              target="_blank"
              rel="noopener noreferrer"
              effect="card"
            >
              <b>{"Source on GitHub"}</b>
              <p className="small" style={{ marginTop: "6px" }}>
                {"Apache-2.0 · self-host with Docker"}
              </p>
              <span className="more" style={{ marginTop: "12px" }}>
                {"Open "}
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
              <b>{"MCP setup guide"}</b>
              <p className="small" style={{ marginTop: "6px" }}>
                {"Read the docs"}
              </p>
              <span className="more" style={{ marginTop: "12px" }}>
                {"Open "}
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
              <b>{"Agent isolation"}</b>
              <p className="small" style={{ marginTop: "6px" }}>
                {"Read the docs"}
              </p>
              <span className="more" style={{ marginTop: "12px" }}>
                {"Open "}
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
              <b>{"Machine nodes"}</b>
              <p className="small" style={{ marginTop: "6px" }}>
                {"Read the docs"}
              </p>
              <span className="more" style={{ marginTop: "12px" }}>
                {"Open "}
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
            <h2 className="h2">
              {"Give your agent access, not your passwords."}
            </h2>
            <p style={{ opacity: ".88", marginTop: "12px", fontSize: "17px" }}>
              {
                "Platform prices, no markup. Open source. Scoped keys from day one."
              }
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
                {"Get started"}
              </a>
              <a className="btn lg gh" href={s.siteHref("/solutions")}>
                {"Browse solutions"}
              </a>
            </div>
          </div>
        </div>
      </section>
    </>
  );
}
