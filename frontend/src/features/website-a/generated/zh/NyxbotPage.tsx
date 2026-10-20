// Generated from approved A, SHA-256 5ba81989df18ad10986c1f00cebdcb1e2efd294932853a7671685187a7e34d33.
// Run: node scripts/website-a/generate.mjs <baseline.html>
import { SiteLink } from "../../site-link";
import { DemoAgents, DemoMessages, DemoChips } from "../../interactions";
import { VisualLink } from "../../visuals/elements";
import { useWebsite } from "../../website-context";
export default function NyxbotPage() {
  const s = useWebsite();
  void s;
  return (
    <>
      {"\n"}
      <section className="s nb-hero" style={{ position: "relative" }}>
        {"\n  "}
        <div className="wrap nb-split">
          {"\n    "}
          <div>
            {"\n      "}
            <div className="nb-tag">
              <span className="nb-av">
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M12 3l1.9 5.8L20 11l-6.1 2.2L12 19l-1.9-5.8L4 11l6.1-2.2z"></path>
                </svg>
              </span>
              {"NyxBot "}
              <span className="small" style={{ marginLeft: "6px" }}>
                {"基于 NyxID"}
              </span>
            </div>
            {"\n      "}
            <h1
              className="h1"
              style={{ fontSize: "var(--fs-display)", marginTop: "22px" }}
            >
              {"直接开口，"}
              <br />
              <span className="grad">{"NyxBot 来办。"}</span>
            </h1>
            {"\n      "}
            <p className="lead" style={{ marginTop: "18px" }}>
              {
                "使用 NyxID 最简单的方式。在应用或 Telegram 里告诉它你需要什么：它会连接你的服务、设定每个助手能做什么，并把重复工作自动化。它不是通用聊天机器人，而是专门为你的应用干活。"
              }
            </p>
            {"\n      "}
            <div className="nb-try">
              <div className="small" style={{ marginBottom: "10px" }}>
                {"试着问它"}
              </div>
              <DemoChips />
            </div>
            {"\n      "}
            <div
              style={{
                display: "flex",
                gap: "12px",
                flexWrap: "wrap",
                marginTop: "30px",
              }}
            >
              <VisualLink
                className="btn p lg"
                href={s.authHref("register")}
                effect="button"
              >
                {"从 NyxBot 开始"}
              </VisualLink>
              <SiteLink className="btn lg" href={s.siteHref("/solutions")}>
                {"看看它能做什么"}
              </SiteLink>
            </div>
            {"\n    "}
          </div>
          {"\n    "}
          <div
            className="nb-app"
            onMouseEnter={s.stopDemo}
            onFocusCapture={s.stopDemo}
          >
            {"\n      "}
            <div className="nb-bar">
              <i></i>
              <i></i>
              <i></i>
              <span>{"NyxID · 助手"}</span>
            </div>
            {"\n      "}
            <div className="nb-body">
              {"\n        "}
              <DemoAgents />
              {"\n        "}
              <div className="nb-thread">
                {"\n          "}
                <div className="nb-th">
                  <span className="nb-av sm">
                    <svg className="i" viewBox="0 0 24 24">
                      <path d="M12 3l1.9 5.8L20 11l-6.1 2.2L12 19l-1.9-5.8L4 11l6.1-2.2z"></path>
                    </svg>
                  </span>
                  <div>
                    <b>{"NyxBot"}</b>
                    <div className="small" style={{ fontSize: "11.5px" }}>
                      {"你的助手 · NyxMode"}
                    </div>
                  </div>
                </div>
                {"\n          "}
                <DemoMessages />
                {"\n          "}
                <div className="nb-comp">
                  <span>{"给 NyxBot 发消息……"}</span>
                  <span className="nb-send">
                    <svg className="i" viewBox="0 0 24 24">
                      <path d="M22 2 11 13"></path>
                      <path d="M22 2 15 22l-4-9-9-4 20-7z"></path>
                    </svg>
                  </span>
                </div>
                {"\n        "}
              </div>
              {"\n      "}
            </div>
            {"\n    "}
          </div>
          {"\n  "}
        </div>
        {"\n"}
      </section>
      {"\n"}
      <section id="try-nyxbot" style={{ padding: "8px 0 24px" }}>
        {"\n  "}
        <div className="wrap">
          <div className="try-strip">
            <span className="try-tag">{"连接 NyxBot"}</span>
            <span className="try-txt">{"登录后连接服务，设置你的助手。"}</span>
            <span className="try-links">
              <SiteLink className="btn sm" href={s.authHref("register")}>
                {"开始使用"}
              </SiteLink>
            </span>
          </div>
        </div>
        {"\n"}
      </section>
      {"\n\n"}
      <section className="s">
        <div className="wrap">
          {"\n  "}
          <div
            className="center"
            style={{
              textAlign: "center",
              maxWidth: "720px",
              marginBottom: "40px",
            }}
          >
            <div className="over v">{"你可以这样说"}</div>
            <h2 className="h2" style={{ marginTop: "10px" }}>
              {"像和同事说话一样。"}
            </h2>
            <p className="lead center" style={{ marginTop: "14px" }}>
              {"不用学菜单。说出你想要的，NyxBot 会搞定服务、权限和计划。"}
            </p>
          </div>
          {"\n  "}
          <div className="grid g2">
            <div className="card say">
              <div className="q">
                {"“我打烊时帮我在 Telegram 上回复客户。”"}
              </div>
              <div className="a">
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M5 12h14M12 5l7 7-7 7"></path>
                </svg>
                <span>
                  {"创建一个客服助手，并绑定到你的 Telegram 机器人。"}
                </span>
              </div>
              <div className="pills" style={{ gap: "6px", marginTop: "14px" }}>
                <span className="badge b-m">{"Telegram"}</span>
                <span className="badge b-m">{"表格 · 只读"}</span>
              </div>
            </div>
            <div className="card say">
              <div className="q">{"“上周我们有多少订位？”"}</div>
              <div className="a">
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M5 12h14M12 5l7 7-7 7"></path>
                </svg>
                <span>{"读取你的日历，回复一份汇总。"}</span>
              </div>
              <div className="pills" style={{ gap: "6px", marginTop: "14px" }}>
                <span className="badge b-m">{"日历 · 只读"}</span>
              </div>
            </div>
            <div className="card say">
              <div className="q">{"“连接我的 Gmail。”"}</div>
              <div className="a">
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M5 12h14M12 5l7 7-7 7"></path>
                </svg>
                <span>{"发送安全登录链接，绝不索要你的密码。"}</span>
              </div>
              <div className="pills" style={{ gap: "6px", marginTop: "14px" }}>
                <span className="badge b-m">{"安全链接"}</span>
              </div>
            </div>
            <div className="card say">
              <div className="q">
                {"“每周一给团队发邮件，列出爽约的客人。”"}
              </div>
              <div className="a">
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M5 12h14M12 5l7 7-7 7"></path>
                </svg>
                <span>{"创建每周计划，第一次发送前先问你。"}</span>
              </div>
              <div className="pills" style={{ gap: "6px", marginTop: "14px" }}>
                <span className="badge b-m">{"日历"}</span>
                <span className="badge b-m">{"Gmail"}</span>
                <span className="badge b-m">{"每周"}</span>
              </div>
            </div>
            <div className="card say">
              <div className="q">{"“从供应商后台下载本月发票。”"}</div>
              <div className="a">
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M5 12h14M12 5l7 7-7 7"></path>
                </svg>
                <span>
                  {"用你电脑上的浏览器操作，已保存的登录信息始终隐藏。"}
                </span>
              </div>
              <div className="pills" style={{ gap: "6px", marginTop: "14px" }}>
                <span className="badge b-m">{"电脑 · 浏览器"}</span>
                <span className="badge b-m">{"文件"}</span>
              </div>
            </div>
            <div className="card say">
              <div className="q">{"“让研究员可以使用 Google 文档。”"}</div>
              <div className="a">
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M5 12h14M12 5l7 7-7 7"></path>
                </svg>
                <span>{"在你批准后，只给这一个助手开放这一个服务。"}</span>
              </div>
              <div className="pills" style={{ gap: "6px", marginTop: "14px" }}>
                <span className="badge b-m">{"授权"}</span>
                <span className="badge b-m">{"需你确认"}</span>
              </div>
            </div>
          </div>
          {"\n"}
        </div>
      </section>
      {"\n\n"}
      <section className="s glowbg">
        {"\n  "}
        <div className="wrap split">
          {"\n    "}
          <div>
            {"\n      "}
            <div className="over v">{"电脑操作"}</div>
            {"\n      "}
            <h2 className="h2" style={{ marginTop: "10px" }}>
              {"没有 API？"}
              <span className="grad">{"那就用电脑。"}</span>
            </h2>
            {"\n      "}
            <p className="lead" style={{ marginTop: "14px" }}>
              {
                "供应商后台、政府表单、老旧的预订系统：很多工作只能在浏览器里完成。连接一台电脑，NyxBot 就能像坐在桌前的人一样点击、输入、下载和归档。"
              }
            </p>
            {"\n      "}
            <div className="grid g2" style={{ marginTop: "26px" }}>
              {"\n        "}
              <div className="card" style={{ padding: "18px" }}>
                <div className="ic">
                  <svg className="i" viewBox="0 0 24 24">
                    <rect x="2" y="3" width="20" height="14" rx="2"></rect>
                    <path d="M8 21h8M12 17v4"></path>
                  </svg>
                </div>
                <b style={{ display: "block", margin: "12px 0 4px" }}>
                  {"你自己的电脑"}
                </b>
                <p className="small" style={{ fontSize: "13.5px" }}>
                  {
                    "办公室里的 Mac 或 Linux 电脑。安装一次 NyxID 应用即可安全连接，无需开放端口。"
                  }
                </p>
              </div>
              {"\n        "}
              <div className="card" style={{ padding: "18px" }}>
                <div className="ic">
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M17.5 19H9a7 7 0 1 1 6.71-9h1.79a4.5 4.5 0 1 1 0 9Z"></path>
                  </svg>
                </div>
                <b style={{ display: "block", margin: "12px 0 4px" }}>
                  {"托管虚拟机"}
                </b>
                <p className="small" style={{ fontSize: "13.5px" }}>
                  {"云端独立桌面，与你自己的文件隔离。最适合全天候运行的工作。"}
                </p>
              </div>
              {"\n      "}
            </div>
            {"\n    "}
          </div>
          {"\n    "}
          <div className="desk mock">
            {"\n  "}
            <div className="tb">
              <i></i>
              <i></i>
              <i></i>
              <span style={{ marginLeft: "8px" }}>{"Mac mini · 实时桌面"}</span>
              <span className="live">{"NyxBot 正在操作"}</span>
            </div>
            {"\n  "}
            <div className="addr">
              <svg className="i" viewBox="0 0 24 24">
                <rect x="3" y="11" width="18" height="11" rx="2"></rect>
                <path d="M7 11V7a5 5 0 0 1 10 0v4"></path>
              </svg>
              {"portal.freshfarms.example/orders/new"}
            </div>
            {"\n  "}
            <div className="pg">
              <h4>{"Fresh Farms · 新订单"}</h4>
              {"\n    "}
              <div className="line">
                <span>{"罗马番茄（箱）"}</span>
                <span>{"× 12"}</span>
                <span>{"$186.00"}</span>
              </div>
              <div className="line">
                <span>{"罗勒（把）"}</span>
                <span>{"× 6"}</span>
                <span>{"$21.00"}</span>
              </div>
              <div className="line">
                <span>{"配送 · 周四上午"}</span>
                <span></span>
                <span>{"免费"}</span>
              </div>
              {"\n    "}
              <div className="tot">
                <b>{"合计 $207.00"}</b>
                <span className="btn p sm">{"下单"}</span>
              </div>
              {"\n  "}
            </div>
            {"\n  "}
            <svg className="cursor" viewBox="0 0 24 24">
              <path
                d="M4 3l7.07 17 2.51-7.39L21 10.07z"
                fill="currentColor"
              ></path>
            </svg>
            {"\n  "}
            <div className="foot">
              <span>
                <svg className="i" viewBox="0 0 24 24">
                  <circle cx="7.5" cy="15.5" r="5.5"></circle>
                  <path d="m21 2-9.6 9.6M15.5 7.5l3 3L22 7l-3-3"></path>
                </svg>
                {" 已填入保存的登录 · 密码对 AI 隐藏"}
              </span>
              <span className="btn sm">{"接管"}</span>
            </div>
            {"\n"}
          </div>
          {"\n  "}
        </div>
        {"\n"}
      </section>
      {"\n\n"}
      <section className="s">
        <div className="wrap">
          {"\n  "}
          <div className="center" style={{ textAlign: "center" }}>
            <div className="over v">{"逐项开启"}</div>
            <h2
              className="h2"
              style={{ marginTop: "10px", marginBottom: "36px" }}
            >
              {"由你决定它能碰什么"}
            </h2>
          </div>
          {"\n  "}
          <div className="grid g4">
            <div className="card">
              <div
                style={{
                  display: "flex",
                  justifyContent: "space-between",
                  alignItems: "flex-start",
                }}
              >
                <div className="ic">
                  <svg className="i" viewBox="0 0 24 24">
                    <rect x="2" y="3" width="20" height="18" rx="2"></rect>
                    <path d="M2 9h20M6 6h.01M9 6h.01"></path>
                  </svg>
                </div>
                <span className="badge b-ok">{"允许"}</span>
              </div>
              <h3 style={{ fontSize: "var(--fs-h3)", margin: "14px 0 6px" }}>
                {"浏览器"}
              </h3>
              <p className="sub" style={{ fontSize: "14px" }}>
                {"在安全浏览器中打开网站、点击并填写表单。"}
              </p>
            </div>
            <div className="card">
              <div
                style={{
                  display: "flex",
                  justifyContent: "space-between",
                  alignItems: "flex-start",
                }}
              >
                <div className="ic">
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M4 20h16a2 2 0 0 0 2-2V8a2 2 0 0 0-2-2h-7.93a2 2 0 0 1-1.66-.9l-.82-1.2A2 2 0 0 0 7.93 3H4a2 2 0 0 0-2 2v13c0 1.1.9 2 2 2Z"></path>
                  </svg>
                </div>
                <span className="badge b-ok">{"允许"}</span>
              </div>
              <h3 style={{ fontSize: "var(--fs-h3)", margin: "14px 0 6px" }}>
                {"文件"}
              </h3>
              <p className="sub" style={{ fontSize: "14px" }}>
                {"只在你指定的文件夹内读取和保存文件。"}
              </p>
            </div>
            <div className="card">
              <div
                style={{
                  display: "flex",
                  justifyContent: "space-between",
                  alignItems: "flex-start",
                }}
              >
                <div className="ic">
                  <svg className="i" viewBox="0 0 24 24">
                    <rect x="2" y="3" width="20" height="14" rx="2"></rect>
                    <path d="M8 21h8M12 17v4"></path>
                  </svg>
                </div>
                <span className="badge b-ask">{"先问我"}</span>
              </div>
              <h3 style={{ fontSize: "var(--fs-h3)", margin: "14px 0 6px" }}>
                {"桌面控制"}
              </h3>
              <p className="sub" style={{ fontSize: "14px" }}>
                {"需要时用鼠标和键盘操作桌面应用。"}
              </p>
            </div>
            <div className="card">
              <div
                style={{
                  display: "flex",
                  justifyContent: "space-between",
                  alignItems: "flex-start",
                }}
              >
                <div className="ic">
                  <svg className="i" viewBox="0 0 24 24">
                    <polyline points="4 17 10 11 4 5"></polyline>
                    <line x1="12" y1="19" x2="20" y2="19"></line>
                  </svg>
                </div>
                <span className="badge b-no">{"禁止"}</span>
              </div>
              <h3 style={{ fontSize: "var(--fs-h3)", margin: "14px 0 6px" }}>
                {"命令行"}
              </h3>
              <p className="sub" style={{ fontSize: "14px" }}>
                {"运行命令。除非你开启，否则关闭。"}
              </p>
            </div>
          </div>
          {"\n  "}
          <div className="grid g3" style={{ marginTop: "16px" }}>
            <div className="card">
              <div className="ic">
                <svg className="i" viewBox="0 0 24 24">
                  <circle cx="7.5" cy="15.5" r="5.5"></circle>
                  <path d="m21 2-9.6 9.6M15.5 7.5l3 3L22 7l-3-3"></path>
                </svg>
              </div>
              <h3 style={{ fontSize: "var(--fs-h3)", margin: "14px 0 6px" }}>
                {"保存的登录信息始终隐藏"}
              </h3>
              <p className="sub" style={{ fontSize: "14px" }}>
                {
                  "密码加密保存，并直接填入指定网站。AI 能操作页面，但读不到密码。"
                }
              </p>
            </div>
            <div className="card">
              <div className="ic">
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M2 12s3-7 10-7 10 7 10 7-3 7-10 7-10-7-10-7Z"></path>
                  <circle cx="12" cy="12" r="3"></circle>
                </svg>
              </div>
              <h3 style={{ fontSize: "var(--fs-h3)", margin: "14px 0 6px" }}>
                {"实时观看，随时接管"}
              </h3>
              <p className="sub" style={{ fontSize: "14px" }}>
                {"在浏览器中打开实时桌面，一键即可接管。"}
              </p>
            </div>
            <div className="card">
              <div className="ic">
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M6 8a6 6 0 0 1 12 0c0 7 3 9 3 9H3s3-2 3-9"></path>
                  <path d="M10.3 21a1.94 1.94 0 0 0 3.4 0"></path>
                </svg>
              </div>
              <h3 style={{ fontSize: "var(--fs-h3)", margin: "14px 0 6px" }}>
                {"操作前先确认"}
              </h3>
              <p className="sub" style={{ fontSize: "14px" }}>
                {"下单、提交表单和修改文件都会等你确认。"}
              </p>
            </div>
          </div>
          {"\n  "}
          <div style={{ textAlign: "center", marginTop: "28px" }}>
            <SiteLink
              className="btn"
              href={s.siteHref("/solutions/back-office-clerk")}
            >
              {"查看“后台文员”方案 "}
              <svg className="i" viewBox="0 0 24 24">
                <path d="M5 12h14M12 5l7 7-7 7"></path>
              </svg>
            </SiteLink>
          </div>
          {"\n"}
        </div>
      </section>
      {"\n\n"}
      <section className="s glowbg">
        <div className="wrap">
          {"\n  "}
          <div
            className="center"
            style={{ textAlign: "center", maxWidth: "720px" }}
          >
            <div className="over v">{"NyxBot + 专属助手"}</div>
            <h2 className="h2" style={{ marginTop: "10px" }}>
              {"一支团队，而非单个机器人"}
            </h2>
            <p className="lead center" style={{ marginTop: "14px" }}>
              {
                "你只需和 NyxBot 对话。每个专属助手只获得它的工作所需的服务：餐厅经理能订桌，但读不了你的邮件。"
              }
            </p>
          </div>
          {"\n  "}
          <div className="org">
            {"\n    "}
            <div className="org-top">
              <span className="nb-av">
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M12 3l1.9 5.8L20 11l-6.1 2.2L12 19l-1.9-5.8L4 11l6.1-2.2z"></path>
                </svg>
              </span>
              <div>
                <b>{"NyxBot"}</b>
                <div className="small">{"与你对话 · 创建并管理专属助手"}</div>
              </div>
            </div>
            {"\n    "}
            <div className="org-stem"></div>
            <div className="org-rail"></div>
            {"\n    "}
            <div className="grid g4 org-grid">
              <VisualLink
                className="card clk"
                href={s.siteHref("/solutions/restaurant-manager")}
                effect="card"
              >
                <div className="ic">
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M3 2v7c0 1.1.9 2 2 2h4a2 2 0 0 0 2-2V2"></path>
                    <path d="M7 2v20"></path>
                    <path d="M21 15V2a5 5 0 0 0-5 5v6c0 1.1.9 2 2 2h3Zm0 0v7"></path>
                  </svg>
                </div>
                <b
                  style={{
                    display: "block",
                    margin: "12px 0 10px",
                    fontSize: "15px",
                  }}
                >
                  {"餐厅经理"}
                </b>
                <div className="pills" style={{ gap: "6px" }}>
                  <span className="badge b-m">{"Telegram"}</span>
                  <span className="badge b-m">{"Google 日历"}</span>
                  <span className="badge b-m">{"Google 表格"}</span>
                </div>
                <div
                  className="small"
                  style={{ marginTop: "12px", color: "var(--err)" }}
                >
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M18 6 6 18M6 6l12 12"></path>
                  </svg>
                  {" 不能：收取订金"}
                </div>
              </VisualLink>
              <VisualLink
                className="card clk"
                href={s.siteHref("/solutions/news-aggregator")}
                effect="card"
              >
                <div className="ic">
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M4 22h16a2 2 0 0 0 2-2V4a2 2 0 0 0-2-2H8a2 2 0 0 0-2 2v16a2 2 0 0 1-2 2Zm0 0a2 2 0 0 1-2-2v-9c0-1.1.9-2 2-2h2"></path>
                    <path d="M18 14h-8"></path>
                    <path d="M15 18h-5"></path>
                    <path d="M10 6h8v4h-8V6Z"></path>
                  </svg>
                </div>
                <b
                  style={{
                    display: "block",
                    margin: "12px 0 10px",
                    fontSize: "15px",
                  }}
                >
                  {"新闻聚合"}
                </b>
                <div className="pills" style={{ gap: "6px" }}>
                  <span className="badge b-m">{"Firecrawl"}</span>
                  <span className="badge b-m">{"X（推特）"}</span>
                  <span className="badge b-m">{"Tavily"}</span>
                  <span className="badge b-m">{"Telegram"}</span>
                </div>
                <div className="small" style={{ marginTop: "12px" }}>
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M2 12s3-7 10-7 10 7 10 7-3 7-10 7-10-7-10-7Z"></path>
                    <circle cx="12" cy="12" r="3"></circle>
                  </svg>
                  {" 只读来源"}
                </div>
              </VisualLink>
              <VisualLink
                className="card clk"
                href={s.siteHref("/solutions/back-office-clerk")}
                effect="card"
              >
                <div className="ic">
                  <svg className="i" viewBox="0 0 24 24">
                    <rect x="2" y="3" width="20" height="14" rx="2"></rect>
                    <path d="M8 21h8M12 17v4"></path>
                  </svg>
                </div>
                <b
                  style={{
                    display: "block",
                    margin: "12px 0 10px",
                    fontSize: "15px",
                  }}
                >
                  {"后台文员"}
                </b>
                <div className="pills" style={{ gap: "6px" }}>
                  <span className="badge b-m">{"电脑 · 浏览器"}</span>
                  <span className="badge b-m">{"电脑 · 文件"}</span>
                  <span className="badge b-m">{"Google 表格"}</span>
                  <span className="badge b-m">{"电脑 · 结账"}</span>
                </div>
                <div
                  className="small"
                  style={{ marginTop: "12px", color: "var(--err)" }}
                >
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M18 6 6 18M6 6l12 12"></path>
                  </svg>
                  {" 不能：运行命令"}
                </div>
              </VisualLink>
              <VisualLink
                className="card clk"
                href={s.siteHref("/solutions/callback-queue")}
                effect="card"
              >
                <div className="ic">
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M22 16.92v3a2 2 0 0 1-2.18 2 19.79 19.79 0 0 1-8.63-3.07 19.5 19.5 0 0 1-6-6 19.79 19.79 0 0 1-3.07-8.67A2 2 0 0 1 4.11 2h3a2 2 0 0 1 2 1.72c.13.96.36 1.9.7 2.81a2 2 0 0 1-.45 2.11L8.09 9.91a16 16 0 0 0 6 6l1.27-1.27a2 2 0 0 1 2.11-.45c.91.34 1.85.57 2.81.7A2 2 0 0 1 22 16.92z"></path>
                  </svg>
                </div>
                <b
                  style={{
                    display: "block",
                    margin: "12px 0 10px",
                    fontSize: "15px",
                  }}
                >
                  {"回访队列"}
                </b>
                <div className="pills" style={{ gap: "6px" }}>
                  <span className="badge b-m">{"Google 表格"}</span>
                  <span className="badge b-m">{"Twilio"}</span>
                  <span className="badge b-m">{"ElevenLabs"}</span>
                  <span className="badge b-m">{"折扣与退款"}</span>
                </div>
                <div
                  className="small"
                  style={{ marginTop: "12px", color: "var(--warn)" }}
                >
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M6 8a6 6 0 0 1 12 0c0 7 3 9 3 9H3s3-2 3-9"></path>
                    <path d="M10.3 21a1.94 1.94 0 0 0 3.4 0"></path>
                  </svg>
                  {" 先问你：提供退款或优惠"}
                </div>
              </VisualLink>
            </div>
            {"\n  "}
          </div>
          {"\n"}
        </div>
      </section>
      {"\n"}
      <section className="s">
        <div className="wrap" style={{ maxWidth: "860px" }}>
          <div className="over v">{"常见问题"}</div>
          <h2 className="h2" style={{ margin: "10px 0 18px" }}>
            {"关于 NyxBot"}
          </h2>
          <details>
            <summary>{"NyxBot 是要取代 ChatGPT、Claude 或 Grok 吗？"}</summary>
            <p>
              {
                "不是。问问题、写东西，继续用你喜欢的 AI。NyxBot 用来运行你的服务：连接、掌控和自动化。你甚至可以把 ChatGPT 或 Claude 接入 NyxID。"
              }
            </p>
          </details>
          <details>
            <summary>{"NyxBot 和 NyxID 有什么区别？"}</summary>
            <p>
              {
                "NyxID 是产品本身：负责登录、限权密钥、审批和日志的安全网关。NyxBot 是用大白话使用它的友好方式。开发者也可以通过 API 或 MCP 直接使用同一个网关。"
              }
            </p>
          </details>
          <details>
            <summary>{"我的客户能和 NyxBot 对话吗？"}</summary>
            <p>
              {
                "客户与你绑定到 Telegram 或 X 机器人的专属助手对话，只能获得你为访客开放的权限，绝不能使用你的电脑。"
              }
            </p>
          </details>
          <details>
            <summary>{"使用我的电脑需要什么？"}</summary>
            <p>
              {
                "在电脑上安装 NyxID 应用（或使用托管虚拟机），与账号配对，然后只开启你需要的功能。"
              }
            </p>
          </details>
          <details>
            <summary>{"能用我自己的 AI 智能体吗？"}</summary>
            <p>
              {"可以。通过 API 或 MCP 连接任何智能体，都能获得同样安全的访问。"}
            </p>
          </details>
        </div>
      </section>
      {"\n"}
      <section className="s">
        <div className="wrap">
          <div className="band">
            <h2 className="h2">{"和 NyxBot 打个招呼吧。"}</h2>
            <p style={{ opacity: ".88", marginTop: "12px", fontSize: "17px" }}>
              {"开始使用，让它为你设置第一项工作。"}
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
    </>
  );
}
