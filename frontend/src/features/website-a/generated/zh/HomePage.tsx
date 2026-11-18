// Generated from approved A, SHA-256 5ba81989df18ad10986c1f00cebdcb1e2efd294932853a7671685187a7e34d33.
// Run: node scripts/website-a/generate.mjs <baseline.html>
import { SiteLink } from "../../site-link";
import {
  AudienceSwitcher,
  CodeExample,
  CommunityAccess,
} from "../../interactions";
import { VisualLink, RevealFrame } from "../../visuals/elements";
import { HeroSection, HeroTitle } from "../../visuals/hero";
import { useWebsite } from "../../website-context";
export default function HomePage() {
  const s = useWebsite();
  void s;
  return (
    <>
      {"\n"}
      <HeroSection
        className="hero glowbg"
        style={{ position: "relative", paddingBottom: "56px" }}
      >
        {"\n  "}
        <div className="wrap">
          {"\n    "}
          <HeroTitle />
          {"\n    "}
          <p className="lead">
            {
              "把无聊的事交给 AI，你专注真正重要的事。连接你的应用，决定它能碰什么，然后让它开工。"
            }
          </p>
          {"\n    "}
          <AudienceSwitcher
            items={[
              {
                id: "business",
                label: (
                  <>
                    <svg className="i" viewBox="0 0 24 24">
                      <path d="M3 2v7c0 1.1.9 2 2 2h4a2 2 0 0 0 2-2V2"></path>
                      <path d="M7 2v20"></path>
                      <path d="M21 15V2a5 5 0 0 0-5 5v6c0 1.1.9 2 2 2h3Zm0 0v7"></path>
                    </svg>
                    {" 给企业"}
                  </>
                ),
                content: (
                  <>
                    <div className="icp-progress" aria-hidden="true">
                      <span></span>
                    </div>
                    {"\n      "}
                    <div className="icp-copy">
                      {"\n        "}
                      <h2
                        className="h3"
                        style={{ fontSize: "28px", letterSpacing: "-.02em" }}
                      >
                        {"一个开箱即用的助手"}
                      </h2>
                      {"\n        "}
                      <ul
                        className="checks"
                        style={{ marginTop: "20px", gap: "14px" }}
                      >
                        <li style={{ "--i": "0" } as React.CSSProperties}>
                          <span className="ck">
                            <svg className="i" viewBox="0 0 24 24">
                              <path d="M20 6 9 17l-5-5"></path>
                            </svg>
                          </span>
                          <div>
                            <span
                              style={{ color: "var(--fg)", fontSize: "15.5px" }}
                            >
                              {"在 Telegram 和 X 上全天候回复客户"}
                            </span>
                          </div>
                        </li>
                        <li style={{ "--i": "1" } as React.CSSProperties}>
                          <span className="ck">
                            <svg className="i" viewBox="0 0 24 24">
                              <path d="M20 6 9 17l-5-5"></path>
                            </svg>
                          </span>
                          <div>
                            <span
                              style={{ color: "var(--fg)", fontSize: "15.5px" }}
                            >
                              {"替你接受预约、回电客户"}
                            </span>
                          </div>
                        </li>
                        <li style={{ "--i": "2" } as React.CSSProperties}>
                          <span className="ck">
                            <svg className="i" viewBox="0 0 24 24">
                              <path d="M20 6 9 17l-5-5"></path>
                            </svg>
                          </span>
                          <div>
                            <span
                              style={{ color: "var(--fg)", fontSize: "15.5px" }}
                            >
                              {"现成方案，几分钟上线，无需代码"}
                            </span>
                          </div>
                        </li>
                      </ul>
                      {"\n        "}
                      <div
                        className="sec-cta left"
                        style={{ marginTop: "28px" }}
                      >
                        <VisualLink
                          className="btn p"
                          href={s.authHref("register")}
                          effect="button"
                        >
                          {"开始使用"}
                        </VisualLink>
                        <SiteLink
                          className="btn"
                          href={s.siteHref("/solutions")}
                        >
                          {"查看方案 "}
                          <svg className="i" viewBox="0 0 24 24">
                            <path d="M5 12h14M12 5l7 7-7 7"></path>
                          </svg>
                        </SiteLink>
                      </div>
                      {"\n      "}
                    </div>
                    {"\n      "}
                    <div className="icp-visual">
                      <div className="phone">
                        <div className="ph-h">
                          <div className="av">
                            <svg className="i" viewBox="0 0 24 24">
                              <path d="M3 2v7c0 1.1.9 2 2 2h4a2 2 0 0 0 2-2V2"></path>
                              <path d="M7 2v20"></path>
                              <path d="M21 15V2a5 5 0 0 0-5 5v6c0 1.1.9 2 2 2h3Zm0 0v7"></path>
                            </svg>
                          </div>
                          <div>
                            <b style={{ fontSize: "14px" }}>{"Luna Bistro"}</b>
                            <div
                              className="small"
                              style={{ fontSize: "11.5px" }}
                            >
                              {"Telegram · 订位"}
                            </div>
                          </div>
                        </div>
                        <div className="msgs">
                          <div
                            className="m sys"
                            style={
                              {
                                "--i": "1",
                                "--step": "900ms",
                              } as React.CSSProperties
                            }
                          >
                            {"周六 晚上 11:42 · 餐厅已打烊"}
                          </div>
                          <div
                            className="m in"
                            style={
                              {
                                "--i": "2",
                                "--step": "900ms",
                              } as React.CSSProperties
                            }
                          >
                            {
                              "你好，我是 Sarah！明晚 7:30 有 4 人位吗？我们有一位吃素。"
                            }
                          </div>
                          <div
                            className="m tool"
                            style={
                              {
                                "--i": "3",
                                "--step": "900ms",
                              } as React.CSSProperties
                            }
                          >
                            {"✓ 已查日历 · 7:30 有空"}
                          </div>
                          <div
                            className="m tool"
                            style={
                              {
                                "--i": "4",
                                "--step": "900ms",
                              } as React.CSSProperties
                            }
                          >
                            {"✓ 已订位 · 已添加备注"}
                          </div>
                          <div
                            className="m out"
                            style={
                              {
                                "--i": "5",
                                "--step": "900ms",
                              } as React.CSSProperties
                            }
                          >
                            {
                              "订好了！周日晚上 7:30，4 人桌，Sarah 名下。已备注素食，厨房会知道。"
                            }
                          </div>
                        </div>
                      </div>
                    </div>
                    {"\n      "}
                    <div className="icp-try" id="try">
                      <span className="try-tag">{"连接 NyxBot"}</span>
                      <span className="try-txt">
                        {"登录后连接服务，设置你的助手。"}
                      </span>
                      <span className="try-links">
                        <SiteLink
                          className="btn sm"
                          href={s.authHref("register")}
                        >
                          {"开始使用"}
                        </SiteLink>
                      </span>
                    </div>
                    {"\n    "}
                  </>
                ),
              },
              {
                id: "enthusiast",
                label: (
                  <>
                    <svg className="i" viewBox="0 0 24 24">
                      <path d="M12 3l1.9 5.8L20 11l-6.1 2.2L12 19l-1.9-5.8L4 11l6.1-2.2z"></path>
                    </svg>
                    {" 给 AI 爱好者"}
                  </>
                ),
                content: (
                  <>
                    <div className="icp-progress" aria-hidden="true">
                      <span></span>
                    </div>
                    {"\n      "}
                    <div className="icp-copy">
                      {"\n        "}
                      <h2
                        className="h3"
                        style={{ fontSize: "28px", letterSpacing: "-.02em" }}
                      >
                        {"看看你的 AI 能做到什么"}
                      </h2>
                      {"\n        "}
                      <ul
                        className="checks"
                        style={{ marginTop: "20px", gap: "14px" }}
                      >
                        <li style={{ "--i": "0" } as React.CSSProperties}>
                          <span className="ck">
                            <svg className="i" viewBox="0 0 24 24">
                              <path d="M20 6 9 17l-5-5"></path>
                            </svg>
                          </span>
                          <div>
                            <span
                              style={{ color: "var(--fg)", fontSize: "15.5px" }}
                            >
                              {"一句话连接 Gmail、日历、X 等服务"}
                            </span>
                          </div>
                        </li>
                        <li style={{ "--i": "1" } as React.CSSProperties}>
                          <span className="ck">
                            <svg className="i" viewBox="0 0 24 24">
                              <path d="M20 6 9 17l-5-5"></path>
                            </svg>
                          </span>
                          <div>
                            <span
                              style={{ color: "var(--fg)", fontSize: "15.5px" }}
                            >
                              {"为每项交出去的工作打造专属助手"}
                            </span>
                          </div>
                        </li>
                        <li style={{ "--i": "2" } as React.CSSProperties}>
                          <span className="ck">
                            <svg className="i" viewBox="0 0 24 24">
                              <path d="M20 6 9 17l-5-5"></path>
                            </svg>
                          </span>
                          <div>
                            <span
                              style={{ color: "var(--fg)", fontSize: "15.5px" }}
                            >
                              {"没有 API 时，让它操作电脑"}
                            </span>
                          </div>
                        </li>
                      </ul>
                      {"\n        "}
                      <div
                        className="sec-cta left"
                        style={{ marginTop: "28px" }}
                      >
                        <VisualLink
                          className="btn p"
                          href={s.authHref("register")}
                          effect="button"
                        >
                          {"开始使用"}
                        </VisualLink>
                        <SiteLink className="btn" href={s.siteHref("/nyxbot")}>
                          {"认识 NyxBot "}
                          <svg className="i" viewBox="0 0 24 24">
                            <path d="M5 12h14M12 5l7 7-7 7"></path>
                          </svg>
                        </SiteLink>
                      </div>
                      {"\n      "}
                    </div>
                    {"\n      "}
                    <div className="icp-visual">
                      <div className="phone">
                        <div className="ph-h">
                          <div className="av">
                            <svg className="i" viewBox="0 0 24 24">
                              <path d="M12 3l1.9 5.8L20 11l-6.1 2.2L12 19l-1.9-5.8L4 11l6.1-2.2z"></path>
                            </svg>
                          </div>
                          <div>
                            <b style={{ fontSize: "14px" }}>{"NyxBot"}</b>
                            <div
                              className="small"
                              style={{ fontSize: "11.5px" }}
                            >
                              {"你的助手"}
                            </div>
                          </div>
                        </div>
                        <div className="msgs">
                          <div
                            className="m in"
                            style={
                              {
                                "--i": "1",
                                "--step": "900ms",
                              } as React.CSSProperties
                            }
                          >
                            {"每天早上 8 点总结我未读的 Gmail"}
                          </div>
                          <div
                            className="m out"
                            style={
                              {
                                "--i": "2",
                                "--step": "900ms",
                              } as React.CSSProperties
                            }
                          >
                            {"好的。先连接 Gmail，我只能读取，不能发送："}
                          </div>
                          <div
                            className="m tool"
                            style={
                              {
                                "--i": "3",
                                "--step": "900ms",
                              } as React.CSSProperties
                            }
                          >
                            {"🔗 安全登录链接 · Gmail（只读）"}
                          </div>
                          <div
                            className="m tool"
                            style={
                              {
                                "--i": "4",
                                "--step": "900ms",
                              } as React.CSSProperties
                            }
                          >
                            {"✓ Gmail 已连接 · 已设定 8:00"}
                          </div>
                          <div
                            className="m out"
                            style={
                              {
                                "--i": "5",
                                "--step": "900ms",
                              } as React.CSSProperties
                            }
                          >
                            {"好了，明早 8 点发第一份总结。"}
                          </div>
                        </div>
                      </div>
                    </div>
                    {"\n      "}
                    <div className="icp-try">
                      <span className="try-tag">{"连接 NyxBot"}</span>
                      <span className="try-txt">
                        {"登录后连接服务，设置你的助手。"}
                      </span>
                      <span className="try-links">
                        <SiteLink
                          className="btn sm"
                          href={s.authHref("register")}
                        >
                          {"开始使用"}
                        </SiteLink>
                      </span>
                    </div>
                    {"\n    "}
                  </>
                ),
              },
              {
                id: "dev",
                label: (
                  <>
                    <svg className="i" viewBox="0 0 24 24">
                      <polyline points="16 18 22 12 16 6"></polyline>
                      <polyline points="8 6 2 12 8 18"></polyline>
                    </svg>
                    {" 给开发者"}
                  </>
                ),
                content: (
                  <>
                    <div className="icp-progress" aria-hidden="true">
                      <span></span>
                    </div>
                    {"\n      "}
                    <div className="icp-copy">
                      {"\n        "}
                      <h2
                        className="h3"
                        style={{ fontSize: "28px", letterSpacing: "-.02em" }}
                      >
                        {"给智能体钥匙，而不是密码"}
                      </h2>
                      {"\n        "}
                      <ul
                        className="checks"
                        style={{ marginTop: "20px", gap: "14px" }}
                      >
                        <li style={{ "--i": "0" } as React.CSSProperties}>
                          <span className="ck">
                            <svg className="i" viewBox="0 0 24 24">
                              <path d="M20 6 9 17l-5-5"></path>
                            </svg>
                          </span>
                          <div>
                            <span
                              style={{ color: "var(--fg)", fontSize: "15.5px" }}
                            >
                              {
                                "通过 MCP 和 REST 接入 Claude、Cursor、Codex 或你自己的代码"
                              }
                            </span>
                          </div>
                        </li>
                        <li style={{ "--i": "1" } as React.CSSProperties}>
                          <span className="ck">
                            <svg className="i" viewBox="0 0 24 24">
                              <path d="M20 6 9 17l-5-5"></path>
                            </svg>
                          </span>
                          <div>
                            <span
                              style={{ color: "var(--fg)", fontSize: "15.5px" }}
                            >
                              {"每个智能体都有限权密钥、审批和审计日志"}
                            </span>
                          </div>
                        </li>
                        <li style={{ "--i": "2" } as React.CSSProperties}>
                          <span className="ck">
                            <svg className="i" viewBox="0 0 24 24">
                              <path d="M20 6 9 17l-5-5"></path>
                            </svg>
                          </span>
                          <div>
                            <span
                              style={{ color: "var(--fg)", fontSize: "15.5px" }}
                            >
                              {"基于 Apache-2.0 开源，可用 Docker 自托管"}
                            </span>
                          </div>
                        </li>
                      </ul>
                      {"\n        "}
                      <div
                        className="sec-cta left"
                        style={{ marginTop: "28px" }}
                      >
                        <VisualLink
                          className="btn p"
                          href={s.authHref("register")}
                          effect="button"
                        >
                          {"开始使用"}
                        </VisualLink>
                        <SiteLink className="btn" href={s.siteHref("/agents")}>
                          {"接入你的智能体 "}
                          <svg className="i" viewBox="0 0 24 24">
                            <path d="M5 12h14M12 5l7 7-7 7"></path>
                          </svg>
                        </SiteLink>
                      </div>
                      {"\n      "}
                    </div>
                    {"\n      "}
                    <div className="icp-visual">
                      <CodeExample
                        labels={["MCP", "curl"]}
                        snippets={[
                          <>
                            <span className="c">
                              {
                                "// Claude、Cursor、Codex……任何 MCP 客户端都能连接 NyxID"
                              }
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
                                "// 你连接的服务会以工具形式出现，并受此密钥的权限限制。"
                              }
                            </span>
                          </>,
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
                            <span className="c">
                              {"# 允许：calendar create_event"}
                            </span>
                            {"\n"}
                            <span className="k">{"→ 403"}</span>
                            {"      "}
                            <span className="c">
                              {"# 同一密钥尝试 delete_event 时"}
                            </span>
                          </>,
                        ]}
                      />
                    </div>
                    {"\n      "}
                    <div className="icp-try">
                      <span className="try-tag">{"连接 NyxBot"}</span>
                      <span className="try-txt">
                        {"登录后连接服务，设置你的助手。"}
                      </span>
                      <span className="try-links">
                        <SiteLink
                          className="btn sm"
                          href={s.authHref("register")}
                        >
                          {"开始使用"}
                        </SiteLink>
                      </span>
                    </div>
                    {"\n    "}
                  </>
                ),
              },
            ]}
          />
          {"\n    "}
          {"\n  "}
        </div>
        {"\n"}
      </HeroSection>
      {"\n\n"}
      <section className="s" style={{ paddingTop: "24px" }}>
        {"\n  "}
        <div className="wrap">
          {"\n    "}
          <RevealFrame className="grid g3 doors">
            {"\n      "}
            <VisualLink
              className="card eq door"
              href={s.siteHref("/how-it-works")}
              effect="card"
            >
              <div className="door-k">
                <span className="ic">
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M12 22v-5M9 8V2M15 8V2M18 8v5a6 6 0 0 1-12 0V8z"></path>
                  </svg>
                </span>
                <span className="over">{"连接"}</span>
              </div>
              <h3>{"连接你已经在用的应用"}</h3>
              <p>
                {
                  "Google、Telegram、X、Twilio 等 100 多种服务，通过安全登录链接连接。密码始终留在保险库里。"
                }
              </p>
              <div className="door-q">{"“连接我的 Google 日历。”"}</div>
              <span className="more">
                {"连接如何运作 "}
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M5 12h14M12 5l7 7-7 7"></path>
                </svg>
              </span>
            </VisualLink>
            <VisualLink
              className="card eq door"
              href={s.siteHref("/security")}
              effect="card"
            >
              <div className="door-k">
                <span className="ic">
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z"></path>
                  </svg>
                </span>
                <span className="over">{"掌控"}</span>
              </div>
              <h3>{"决定 AI 能碰什么"}</h3>
              <p>
                {
                  "按操作设置允许、先问我或禁止。在手机上批准高风险操作，在一份记录里看到每次调用。"
                }
              </p>
              <div className="door-q">{"“没有我同意，绝不能取消预约。”"}</div>
              <span className="more">
                {"掌控如何运作 "}
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M5 12h14M12 5l7 7-7 7"></path>
                </svg>
              </span>
            </VisualLink>
            <VisualLink
              className="card eq door"
              href={s.siteHref("/solutions")}
              effect="card"
            >
              <div className="door-k">
                <span className="ic">
                  <svg className="i" viewBox="0 0 24 24">
                    <circle cx="12" cy="12" r="10"></circle>
                    <path d="M12 6v6l4 2"></path>
                  </svg>
                </span>
                <span className="over">{"自动化"}</span>
              </div>
              <h3>{"把重复工作交出去"}</h3>
              <p>
                {
                  "接受预约、回复客户、回电客户或发送早报，随叫随到或按计划运行。"
                }
              </p>
              <div className="door-q">
                {"“每个工作日 8 点，把今天的预约发给我。”"}
              </div>
              <span className="more">
                {"查看现成工作 "}
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M5 12h14M12 5l7 7-7 7"></path>
                </svg>
              </span>
            </VisualLink>
            {"\n    "}
          </RevealFrame>
          {"\n    "}
          <div className="sec-cta">
            <VisualLink
              className="btn p"
              href={s.authHref("register")}
              effect="button"
            >
              {"开始使用"}
            </VisualLink>
            <SiteLink className="btn" href={s.siteHref("/how-it-works")}>
              {"了解更多 "}
              <svg className="i" viewBox="0 0 24 24">
                <path d="M5 12h14M12 5l7 7-7 7"></path>
              </svg>
            </SiteLink>
          </div>
          {"\n  "}
        </div>
        {"\n"}
      </section>
      {"\n\n"}
      <section className="s">
        {"\n  "}
        <div className="wrap split">
          {"\n    "}
          <div>
            <div className="over v">{"不是又一个聊天机器人"}</div>
            <h2 className="h2" style={{ marginTop: "10px" }}>
              {"继续用你喜欢的 AI，"}
              <br />
              <span className="grad">{"访问权限交给 NyxID。"}</span>
            </h2>
            {"\n      "}
            <p className="lead" style={{ marginTop: "14px" }}>
              {
                "ChatGPT、Claude 和 Grok 擅长回答问题，NyxBot 并不想取代它们。它负责运行你的服务：那些需要真实权限的预约、回复、文件和通话。"
              }
            </p>
            {"\n      "}
            <p className="sub" style={{ marginTop: "14px" }}>
              {
                "已经在用它们，或在做自己的智能体？把它接入 NyxID，就能获得同样限权、需审批、有记录的访问。"
              }
            </p>
            {"\n      "}
            <div style={{ marginTop: "24px" }}>
              <div className="sec-cta left">
                <VisualLink
                  className="btn p"
                  href={s.authHref("register")}
                  effect="button"
                >
                  {"开始使用"}
                </VisualLink>
                <SiteLink className="btn" href={s.siteHref("/agents")}>
                  {"了解更多 "}
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M5 12h14M12 5l7 7-7 7"></path>
                  </svg>
                </SiteLink>
              </div>
            </div>
          </div>
          {"\n    "}
          <div className="fit fit-v">
            {"\n      "}
            <div className="fit-n ">
              <span className="ic ">
                <svg className="i" viewBox="0 0 24 24">
                  <circle cx="12" cy="8" r="4"></circle>
                  <path d="M4 21v-1a6 6 0 0 1 6-6h4a6 6 0 0 1 6 6v1"></path>
                </svg>
              </span>
              <div>
                <b>{"你，用大白话"}</b>
                <span>{"或你自己的 AI 智能体"}</span>
              </div>
            </div>
            <span className="fit-a">
              <svg className="i" viewBox="0 0 24 24">
                <path d="M5 12h14M12 5l7 7-7 7"></path>
              </svg>
            </span>
            <div className="fit-n ">
              <span className="ic ">
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M12 3l1.9 5.8L20 11l-6.1 2.2L12 19l-1.9-5.8L4 11l6.1-2.2z"></path>
                </svg>
              </span>
              <div>
                <b>{"NyxBot"}</b>
                <span>{"把需求变成步骤"}</span>
              </div>
            </div>
            <span className="fit-a">
              <svg className="i" viewBox="0 0 24 24">
                <path d="M5 12h14M12 5l7 7-7 7"></path>
              </svg>
            </span>
            <div className="fit-n hi">
              <span className="ic g">
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z"></path>
                </svg>
              </span>
              <div>
                <b>{"NyxID"}</b>
                <span>{"检查密钥、征求同意、记录在案"}</span>
              </div>
            </div>
            <span className="fit-a">
              <svg className="i" viewBox="0 0 24 24">
                <path d="M5 12h14M12 5l7 7-7 7"></path>
              </svg>
            </span>
            <div className="fit-n ">
              <span className="ic ">
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M12 22v-5M9 8V2M15 8V2M18 8v5a6 6 0 0 1-12 0V8z"></path>
                </svg>
              </span>
              <div>
                <b>{"你的应用与 API"}</b>
                <span>{"100 多种服务"}</span>
              </div>
            </div>
            {"\n    "}
          </div>
          {"\n  "}
        </div>
        {"\n"}
      </section>
      {"\n\n"}
      <section className="s glowbg">
        {"\n  "}
        <div className="wrap">
          {"\n    "}
          <div
            className="center"
            style={{
              textAlign: "center",
              maxWidth: "720px",
              marginBottom: "36px",
            }}
          >
            <div className="over v">{"集成"}</div>
            <h2 className="h2" style={{ marginTop: "10px" }}>
              {"支持你已经在用的工具"}
            </h2>
            <p className="lead center" style={{ marginTop: "14px" }}>
              {
                "连接一次即可。NyxBot 和你自己的智能体获得限权访问，永远拿不到你的密码。"
              }
            </p>
          </div>
          {"\n    "}
          <div className="int-wall">
            <div
              className="int "
              data-name="notion"
              data-cat="0"
              data-av="1"
              hidden={!s.integrationVisible("notion", "0", "1")}
            >
              <span className="int-logo">
                <img
                  src="https://www.google.com/s2/favicons?domain=notion.so&sz=64"
                  alt=""
                  loading="lazy"
                  width="20"
                  height="20"
                  onError={s.imageFallback}
                />
              </span>
              <span className="int-txt">
                <b>{"Notion"}</b>
              </span>
            </div>
            <div
              className="int "
              data-name="slack"
              data-cat="1"
              data-av="1"
              hidden={!s.integrationVisible("slack", "1", "1")}
            >
              <span className="int-logo">
                <img
                  src="https://www.google.com/s2/favicons?domain=slack.com&sz=64"
                  alt=""
                  loading="lazy"
                  width="20"
                  height="20"
                  onError={s.imageFallback}
                />
              </span>
              <span className="int-txt">
                <b>{"Slack"}</b>
              </span>
            </div>
            <div
              className="int "
              data-name="github"
              data-cat="2"
              data-av="1"
              hidden={!s.integrationVisible("github", "2", "1")}
            >
              <span className="int-logo">
                <img
                  src="https://www.google.com/s2/favicons?domain=github.com&sz=64"
                  alt=""
                  loading="lazy"
                  width="20"
                  height="20"
                  onError={s.imageFallback}
                />
              </span>
              <span className="int-txt">
                <b>{"GitHub"}</b>
              </span>
            </div>
            <div
              className="int "
              data-name="linear"
              data-cat="2"
              data-av="1"
              hidden={!s.integrationVisible("linear", "2", "1")}
            >
              <span className="int-logo">
                <img
                  src="https://www.google.com/s2/favicons?domain=linear.app&sz=64"
                  alt=""
                  loading="lazy"
                  width="20"
                  height="20"
                  onError={s.imageFallback}
                />
              </span>
              <span className="int-txt">
                <b>{"Linear"}</b>
              </span>
            </div>
            <div
              className="int "
              data-name="google sheets"
              data-cat="3"
              data-av="1"
              hidden={!s.integrationVisible("google sheets", "3", "1")}
            >
              <span className="int-logo">
                <img
                  src="https://www.google.com/s2/favicons?domain=google.com&sz=64"
                  alt=""
                  loading="lazy"
                  width="20"
                  height="20"
                  onError={s.imageFallback}
                />
              </span>
              <span className="int-txt">
                <b>{"Google Sheets"}</b>
              </span>
            </div>
            <div
              className="int "
              data-name="gmail"
              data-cat="4"
              data-av="1"
              hidden={!s.integrationVisible("gmail", "4", "1")}
            >
              <span className="int-logo">
                <img
                  src="https://www.google.com/s2/favicons?domain=mail.google.com&sz=64"
                  alt=""
                  loading="lazy"
                  width="20"
                  height="20"
                  onError={s.imageFallback}
                />
              </span>
              <span className="int-txt">
                <b>{"Gmail"}</b>
              </span>
            </div>
            <div
              className="int "
              data-name="google calendar"
              data-cat="5"
              data-av="1"
              hidden={!s.integrationVisible("google calendar", "5", "1")}
            >
              <span className="int-logo">
                <img
                  src="https://www.google.com/s2/favicons?domain=calendar.google.com&sz=64"
                  alt=""
                  loading="lazy"
                  width="20"
                  height="20"
                  onError={s.imageFallback}
                />
              </span>
              <span className="int-txt">
                <b>{"Google Calendar"}</b>
              </span>
            </div>
            <div
              className="int "
              data-name="google drive"
              data-cat="6"
              data-av="1"
              hidden={!s.integrationVisible("google drive", "6", "1")}
            >
              <span className="int-logo">
                <img
                  src="https://www.google.com/s2/favicons?domain=drive.google.com&sz=64"
                  alt=""
                  loading="lazy"
                  width="20"
                  height="20"
                  onError={s.imageFallback}
                />
              </span>
              <span className="int-txt">
                <b>{"Google Drive"}</b>
              </span>
            </div>
            <div
              className="int "
              data-name="asana"
              data-cat="7"
              data-av="1"
              hidden={!s.integrationVisible("asana", "7", "1")}
            >
              <span className="int-logo">
                <img
                  src="https://www.google.com/s2/favicons?domain=asana.com&sz=64"
                  alt=""
                  loading="lazy"
                  width="20"
                  height="20"
                  onError={s.imageFallback}
                />
              </span>
              <span className="int-txt">
                <b>{"Asana"}</b>
              </span>
            </div>
            <div
              className="int "
              data-name="box"
              data-cat="6"
              data-av="1"
              hidden={!s.integrationVisible("box", "6", "1")}
            >
              <span className="int-logo">
                <img
                  src="https://www.google.com/s2/favicons?domain=box.com&sz=64"
                  alt=""
                  loading="lazy"
                  width="20"
                  height="20"
                  onError={s.imageFallback}
                />
              </span>
              <span className="int-txt">
                <b>{"Box"}</b>
              </span>
            </div>
            <div
              className="int "
              data-name="jira"
              data-cat="2"
              data-av="1"
              hidden={!s.integrationVisible("jira", "2", "1")}
            >
              <span className="int-logo">
                <img
                  src="https://www.google.com/s2/favicons?domain=atlassian.com&sz=64"
                  alt=""
                  loading="lazy"
                  width="20"
                  height="20"
                  onError={s.imageFallback}
                />
              </span>
              <span className="int-txt">
                <b>{"Jira"}</b>
              </span>
            </div>
            <div
              className="int "
              data-name="zoom"
              data-cat="9"
              data-av="1"
              hidden={!s.integrationVisible("zoom", "9", "1")}
            >
              <span className="int-logo">
                <img
                  src="https://www.google.com/s2/favicons?domain=zoom.com&sz=64"
                  alt=""
                  loading="lazy"
                  width="20"
                  height="20"
                  onError={s.imageFallback}
                />
              </span>
              <span className="int-txt">
                <b>{"Zoom"}</b>
              </span>
            </div>
            <div
              className="int "
              data-name="dropbox"
              data-cat="6"
              data-av="1"
              hidden={!s.integrationVisible("dropbox", "6", "1")}
            >
              <span className="int-logo">
                <img
                  src="https://www.google.com/s2/favicons?domain=dropbox.com&sz=64"
                  alt=""
                  loading="lazy"
                  width="20"
                  height="20"
                  onError={s.imageFallback}
                />
              </span>
              <span className="int-txt">
                <b>{"Dropbox"}</b>
              </span>
            </div>
            <div
              className="int "
              data-name="hubspot"
              data-cat="8"
              data-av="1"
              hidden={!s.integrationVisible("hubspot", "8", "1")}
            >
              <span className="int-logo">
                <img
                  src="https://www.google.com/s2/favicons?domain=hubspot.com&sz=64"
                  alt=""
                  loading="lazy"
                  width="20"
                  height="20"
                  onError={s.imageFallback}
                />
              </span>
              <span className="int-txt">
                <b>{"HubSpot"}</b>
              </span>
            </div>
            <div
              className="int "
              data-name="clickup"
              data-cat="7"
              data-av="1"
              hidden={!s.integrationVisible("clickup", "7", "1")}
            >
              <span className="int-logo">
                <img
                  src="https://www.google.com/s2/favicons?domain=clickup.com&sz=64"
                  alt=""
                  loading="lazy"
                  width="20"
                  height="20"
                  onError={s.imageFallback}
                />
              </span>
              <span className="int-txt">
                <b>{"ClickUp"}</b>
              </span>
            </div>
            <div
              className="int "
              data-name="google docs"
              data-cat="0"
              data-av="1"
              hidden={!s.integrationVisible("google docs", "0", "1")}
            >
              <span className="int-logo">
                <img
                  src="https://www.google.com/s2/favicons?domain=docs.google.com&sz=64"
                  alt=""
                  loading="lazy"
                  width="20"
                  height="20"
                  onError={s.imageFallback}
                />
              </span>
              <span className="int-txt">
                <b>{"Google Docs"}</b>
              </span>
            </div>
            <div
              className="int "
              data-name="airtable"
              data-cat="3"
              data-av="1"
              hidden={!s.integrationVisible("airtable", "3", "1")}
            >
              <span className="int-logo">
                <img
                  src="https://www.google.com/s2/favicons?domain=airtable.com&sz=64"
                  alt=""
                  loading="lazy"
                  width="20"
                  height="20"
                  onError={s.imageFallback}
                />
              </span>
              <span className="int-txt">
                <b>{"Airtable"}</b>
              </span>
            </div>
            <div
              className="int "
              data-name="microsoft outlook (mail & calendar)"
              data-cat="4"
              data-av="1"
              hidden={
                !s.integrationVisible(
                  "microsoft outlook (mail & calendar)",
                  "4",
                  "1",
                )
              }
            >
              <span className="int-logo">
                <img
                  src="https://www.google.com/s2/favicons?domain=learn.microsoft.com&sz=64"
                  alt=""
                  loading="lazy"
                  width="20"
                  height="20"
                  onError={s.imageFallback}
                />
              </span>
              <span className="int-txt">
                <b>{"Microsoft Outlook (Mail & Calendar)"}</b>
              </span>
            </div>
            <div
              className="int "
              data-name="stripe"
              data-cat="11"
              data-av="1"
              hidden={!s.integrationVisible("stripe", "11", "1")}
            >
              <span className="int-logo">
                <img
                  src="https://www.google.com/s2/favicons?domain=stripe.com&sz=64"
                  alt=""
                  loading="lazy"
                  width="20"
                  height="20"
                  onError={s.imageFallback}
                />
              </span>
              <span className="int-txt">
                <b>{"Stripe"}</b>
              </span>
            </div>
            <div
              className="int "
              data-name="microsoft onedrive"
              data-cat="6"
              data-av="1"
              hidden={!s.integrationVisible("microsoft onedrive", "6", "1")}
            >
              <span className="int-logo">
                <img
                  src="https://www.google.com/s2/favicons?domain=onedrive.live.com&sz=64"
                  alt=""
                  loading="lazy"
                  width="20"
                  height="20"
                  onError={s.imageFallback}
                />
              </span>
              <span className="int-txt">
                <b>{"Microsoft OneDrive"}</b>
              </span>
            </div>
            <div
              className="int "
              data-name="calendly"
              data-cat="5"
              data-av="1"
              hidden={!s.integrationVisible("calendly", "5", "1")}
            >
              <span className="int-logo">
                <img
                  src="https://www.google.com/s2/favicons?domain=calendly.com&sz=64"
                  alt=""
                  loading="lazy"
                  width="20"
                  height="20"
                  onError={s.imageFallback}
                />
              </span>
              <span className="int-txt">
                <b>{"Calendly"}</b>
              </span>
            </div>
            <div
              className="int "
              data-name="figma"
              data-cat="12"
              data-av="1"
              hidden={!s.integrationVisible("figma", "12", "1")}
            >
              <span className="int-logo">
                <img
                  src="https://www.google.com/s2/favicons?domain=figma.com&sz=64"
                  alt=""
                  loading="lazy"
                  width="20"
                  height="20"
                  onError={s.imageFallback}
                />
              </span>
              <span className="int-txt">
                <b>{"Figma"}</b>
              </span>
            </div>
            <div
              className="int "
              data-name="microsoft teams"
              data-cat="1"
              data-av="1"
              hidden={!s.integrationVisible("microsoft teams", "1", "1")}
            >
              <span className="int-logo">
                <img
                  src="https://www.google.com/s2/favicons?domain=teams.microsoft.com&sz=64"
                  alt=""
                  loading="lazy"
                  width="20"
                  height="20"
                  onError={s.imageFallback}
                />
              </span>
              <span className="int-txt">
                <b>{"Microsoft Teams"}</b>
              </span>
            </div>
            <div
              className="int "
              data-name="intercom"
              data-cat="10"
              data-av="1"
              hidden={!s.integrationVisible("intercom", "10", "1")}
            >
              <span className="int-logo">
                <img
                  src="https://www.google.com/s2/favicons?domain=intercom.com&sz=64"
                  alt=""
                  loading="lazy"
                  width="20"
                  height="20"
                  onError={s.imageFallback}
                />
              </span>
              <span className="int-txt">
                <b>{"Intercom"}</b>
              </span>
            </div>
          </div>
          {"\n    "}
          <p
            className="small center"
            style={{ textAlign: "center", marginTop: "18px" }}
          >
            {"56 个已上线 · 89 个即将推出"}
          </p>
          {"\n    "}
          <div className="sec-cta">
            <VisualLink
              className="btn p"
              href={s.authHref("register")}
              effect="button"
            >
              {"开始使用"}
            </VisualLink>
            <SiteLink className="btn" href={s.siteHref("/integrations")}>
              {"查看全部集成 "}
              <svg className="i" viewBox="0 0 24 24">
                <path d="M5 12h14M12 5l7 7-7 7"></path>
              </svg>
            </SiteLink>
          </div>
          {"\n  "}
        </div>
        {"\n"}
      </section>
      {"\n"}
      <section className="s">
        <div className="wrap">
          <div className="band">
            <h2 className="h2">{"今天就从一项工作开始。"}</h2>
            <p style={{ opacity: ".88", marginTop: "12px", fontSize: "17px" }}>
              {"平台原价，零加价。可免费自托管。"}
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
              <SiteLink className="btn lg" href={s.authHref("register")}>
                {"开始使用"}
              </SiteLink>
              <SiteLink className="btn lg gh" href={s.siteHref("/solutions")}>
                {"浏览方案"}
              </SiteLink>
            </div>
          </div>
        </div>
      </section>
      <CommunityAccess />
    </>
  );
}
