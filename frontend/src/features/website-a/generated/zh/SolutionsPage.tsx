// Generated from approved A, SHA-256 5ba81989df18ad10986c1f00cebdcb1e2efd294932853a7671685187a7e34d33.
// Run: node scripts/website-a/generate.mjs <baseline.html>
import { SolutionFilm } from "../../interactions";
import { VisualLink } from "../../visuals/elements";
import { useWebsite } from "../../website-context";
export default function SolutionsPage() {
  const s = useWebsite();
  void s;
  return (
    <>
      {"\n"}
      <section
        className="hero glowbg"
        style={{ paddingBottom: "40px", position: "relative" }}
      >
        {"\n  "}
        <div className="wrap">
          <div className="over v">{"方案"}</div>
          <h1
            className="h1"
            style={{ marginTop: "14px", fontSize: "var(--fs-h1)" }}
          >
            {"你的助手要做什么？"}
          </h1>
          <p className="lead">
            {"为真实业务准备的现成工作，已配好合适的连接和安全的权限。"}
          </p>
          <div
            className="pills"
            style={{ marginTop: "22px", justifyContent: "center" }}
          >
            <span className="badge b-ok">{"● 平台原价，零加价"}</span>
            <span className="badge b-v">{"企业定制套餐"}</span>
          </div>
        </div>
        {"\n"}
      </section>
      {"\n"}
      <section className="s" style={{ paddingTop: "8px", border: "none" }}>
        <div className="wrap">
          <div className="center" style={{ textAlign: "center" }}>
            <div className="over v">{"看看效果"}</div>
            <h2 className="h2" style={{ marginTop: "10px" }}>
              {"五种工作，一个助手"}
            </h2>
          </div>
          <SolutionFilm />
        </div>
      </section>
      {"\n"}
      <section className="s" style={{ paddingTop: "40px" }}>
        {"\n  "}
        <div className="wrap">
          {"\n    "}
          <div
            style={{
              display: "flex",
              justifyContent: "space-between",
              gap: "16px",
              flexWrap: "wrap",
              marginBottom: "28px",
            }}
          >
            {"\n      "}
            <div className="pills" id="catF">
              <button
                className={s.filterClass("catF", "all")}
                data-v="all"
                type="button"
                onClick={() => s.setFilter("catF", "all")}
                aria-pressed={s.isFilterSelected("catF", "all")}
              >
                {"全部"}
              </button>
              <button
                className={s.filterClass("catF", "hosp")}
                data-v="hosp"
                type="button"
                onClick={() => s.setFilter("catF", "hosp")}
                aria-pressed={s.isFilterSelected("catF", "hosp")}
              >
                {"餐饮与服务"}
              </button>
              <button
                className={s.filterClass("catF", "support")}
                data-v="support"
                type="button"
                onClick={() => s.setFilter("catF", "support")}
                aria-pressed={s.isFilterSelected("catF", "support")}
              >
                {"客户支持"}
              </button>
              <button
                className={s.filterClass("catF", "research")}
                data-v="research"
                type="button"
                onClick={() => s.setFilter("catF", "research")}
                aria-pressed={s.isFilterSelected("catF", "research")}
              >
                {"研究与洞察"}
              </button>
              <button
                className={s.filterClass("catF", "sales")}
                data-v="sales"
                type="button"
                onClick={() => s.setFilter("catF", "sales")}
                aria-pressed={s.isFilterSelected("catF", "sales")}
              >
                {"销售与营销"}
              </button>
              <button
                className={s.filterClass("catF", "ops")}
                data-v="ops"
                type="button"
                onClick={() => s.setFilter("catF", "ops")}
                aria-pressed={s.isFilterSelected("catF", "ops")}
              >
                {"运营"}
              </button>
            </div>
            {"\n      "}
            <div className="pills" id="chF">
              <span
                className="small"
                style={{ alignSelf: "center", marginRight: "4px" }}
              >
                {"渠道"}
              </span>
              <button
                className={s.filterClass("chF", "Telegram")}
                data-v="Telegram"
                type="button"
                onClick={() => s.setFilter("chF", "Telegram")}
                aria-pressed={s.isFilterSelected("chF", "Telegram")}
              >
                {"Telegram"}
              </button>
              <button
                className={s.filterClass("chF", "X")}
                data-v="X"
                type="button"
                onClick={() => s.setFilter("chF", "X")}
                aria-pressed={s.isFilterSelected("chF", "X")}
              >
                {"X"}
              </button>
              <button
                className={s.filterClass("chF", "Phone")}
                data-v="Phone"
                type="button"
                onClick={() => s.setFilter("chF", "Phone")}
                aria-pressed={s.isFilterSelected("chF", "Phone")}
              >
                {"电话"}
              </button>
              <button
                className={s.filterClass("chF", "Computer")}
                data-v="Computer"
                type="button"
                onClick={() => s.setFilter("chF", "Computer")}
                aria-pressed={s.isFilterSelected("chF", "Computer")}
              >
                {"电脑"}
              </button>
            </div>
            {"\n    "}
          </div>
          {"\n    "}
          <div className="grid g3" id="solGrid">
            <VisualLink
              className="card sol feat"
              href={s.siteHref("/solutions/restaurant-manager")}
              data-category="hosp"
              data-channels="Telegram"
              hidden={!s.solutionVisible("hosp", "Telegram")}
              effect="card"
            >
              {"\n  "}
              <div className="top">
                <div className="ic">
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M3 2v7c0 1.1.9 2 2 2h4a2 2 0 0 0 2-2V2"></path>
                    <path d="M7 2v20"></path>
                    <path d="M21 15V2a5 5 0 0 0-5 5v6c0 1.1.9 2 2 2h3Zm0 0v7"></path>
                  </svg>
                </div>
                <span className="badge b-v">{"最受欢迎"}</span>
              </div>
              {"\n  "}
              <h3>{"餐厅经理"}</h3>
              <p>
                {"在 Telegram 上接受订位，查看你的日历，记录忌口并发送提醒。"}
              </p>
              {"\n  "}
              <div className="tags">
                <span className="badge b-ok">{"● Telegram"}</span>
              </div>
              {"\n  "}
              <div className="small" style={{ fontSize: "12px" }}>
                {"平台原价 · 零加价"}
              </div>
              {"\n  "}
              <div className="foot">
                <span>
                  <svg className="i" viewBox="0 0 24 24">
                    <rect x="3" y="11" width="18" height="11" rx="2"></rect>
                    <path d="M7 11V7a5 5 0 0 1 10 0v4"></path>
                  </svg>
                  {"禁止：收取订金"}
                </span>
                <span className="more">
                  {"查看 "}
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M5 12h14M12 5l7 7-7 7"></path>
                  </svg>
                </span>
              </div>
            </VisualLink>
            <VisualLink
              className="card sol "
              href={s.siteHref("/solutions/help-desk")}
              data-category="support"
              data-channels="Telegram,X"
              hidden={!s.solutionVisible("support", "Telegram,X")}
              effect="card"
            >
              {"\n  "}
              <div className="top">
                <div className="ic">
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M3 14h3a2 2 0 0 1 2 2v3a2 2 0 0 1-2 2H5a2 2 0 0 1-2-2v-7a9 9 0 0 1 18 0v7a2 2 0 0 1-2 2h-1a2 2 0 0 1-2-2v-3a2 2 0 0 1 2-2h3"></path>
                  </svg>
                </div>
                <span className="badge b-m">{"客户支持"}</span>
              </div>
              {"\n  "}
              <h3>{"24 小时客服"}</h3>
              <p>
                {"随时回答订单、配送和政策问题，把棘手的问题转给你的团队。"}
              </p>
              {"\n  "}
              <div className="tags">
                <span className="badge b-ok">{"● Telegram"}</span>
                <span className="badge b-ok">{"● X"}</span>
              </div>
              {"\n  "}
              <div className="small" style={{ fontSize: "12px" }}>
                {"平台原价 · 零加价"}
              </div>
              {"\n  "}
              <div className="foot">
                <span>
                  <svg className="i" viewBox="0 0 24 24">
                    <rect x="3" y="11" width="18" height="11" rx="2"></rect>
                    <path d="M7 11V7a5 5 0 0 1 10 0v4"></path>
                  </svg>
                  {"禁止：办理退款"}
                </span>
                <span className="more">
                  {"查看 "}
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M5 12h14M12 5l7 7-7 7"></path>
                  </svg>
                </span>
              </div>
            </VisualLink>
            <VisualLink
              className="card sol "
              href={s.siteHref("/solutions/appointment-booker")}
              data-category="hosp"
              data-channels="Telegram"
              hidden={!s.solutionVisible("hosp", "Telegram")}
              effect="card"
            >
              {"\n  "}
              <div className="top">
                <div className="ic">
                  <svg className="i" viewBox="0 0 24 24">
                    <circle cx="6" cy="6" r="3"></circle>
                    <circle cx="6" cy="18" r="3"></circle>
                    <path d="M20 4 8.12 15.88M14.47 14.48 20 20M8.12 8.12 12 12"></path>
                  </svg>
                </div>
                <span className="badge b-m">{"餐饮与服务"}</span>
              </div>
              {"\n  "}
              <h3>{"预约助手"}</h3>
              <p>{"提供可预约时段，约到合适的人员，改期需经你确认。"}</p>
              {"\n  "}
              <div className="tags">
                <span className="badge b-ok">{"● Telegram"}</span>
              </div>
              {"\n  "}
              <div className="small" style={{ fontSize: "12px" }}>
                {"平台原价 · 零加价"}
              </div>
              {"\n  "}
              <div className="foot">
                <span>
                  <svg className="i" viewBox="0 0 24 24">
                    <rect x="3" y="11" width="18" height="11" rx="2"></rect>
                    <path d="M7 11V7a5 5 0 0 1 10 0v4"></path>
                  </svg>
                  {"禁止：调整预约"}
                </span>
                <span className="more">
                  {"查看 "}
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M5 12h14M12 5l7 7-7 7"></path>
                  </svg>
                </span>
              </div>
            </VisualLink>
            <VisualLink
              className="card sol "
              href={s.siteHref("/solutions/callback-queue")}
              data-category="support"
              data-channels="Phone"
              hidden={!s.solutionVisible("support", "Phone")}
              effect="card"
            >
              {"\n  "}
              <div className="top">
                <div className="ic">
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M22 16.92v3a2 2 0 0 1-2.18 2 19.79 19.79 0 0 1-8.63-3.07 19.5 19.5 0 0 1-6-6 19.79 19.79 0 0 1-3.07-8.67A2 2 0 0 1 4.11 2h3a2 2 0 0 1 2 1.72c.13.96.36 1.9.7 2.81a2 2 0 0 1-.45 2.11L8.09 9.91a16 16 0 0 0 6 6l1.27-1.27a2 2 0 0 1 2.11-.45c.91.34 1.85.57 2.81.7A2 2 0 0 1 22 16.92z"></path>
                  </svg>
                </div>
                <span className="badge b-m">{"客户支持"}</span>
              </div>
              {"\n  "}
              <h3>{"回访队列"}</h3>
              <p>{"按名单逐一回电客户，用 AI 语音完成通话，并记录结果。"}</p>
              {"\n  "}
              <div className="tags">
                <span className="badge b-ro">{"● 电话"}</span>
              </div>
              {"\n  "}
              <div className="small" style={{ fontSize: "12px" }}>
                {"平台原价 · 零加价"}
              </div>
              {"\n  "}
              <div className="foot">
                <span>
                  <svg className="i" viewBox="0 0 24 24">
                    <rect x="3" y="11" width="18" height="11" rx="2"></rect>
                    <path d="M7 11V7a5 5 0 0 1 10 0v4"></path>
                  </svg>
                  {"禁止：提供退款或优惠"}
                </span>
                <span className="more">
                  {"查看 "}
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M5 12h14M12 5l7 7-7 7"></path>
                  </svg>
                </span>
              </div>
            </VisualLink>
            <VisualLink
              className="card sol "
              href={s.siteHref("/solutions/stock-analyst")}
              data-category="research"
              data-channels="Telegram"
              hidden={!s.solutionVisible("research", "Telegram")}
              effect="card"
            >
              {"\n  "}
              <div className="top">
                <div className="ic">
                  <svg className="i" viewBox="0 0 24 24">
                    <polyline points="22 7 13.5 15.5 8.5 10.5 2 17"></polyline>
                    <polyline points="16 7 22 7 22 13"></polyline>
                  </svg>
                </div>
                <span className="badge b-m">{"研究与洞察"}</span>
              </div>
              {"\n  "}
              <h3>{"股票分析师"}</h3>
              <p>
                {
                  "盯住你的自选股，扫描市场讨论，用大白话给你简报。只读，从不交易。"
                }
              </p>
              {"\n  "}
              <div className="tags">
                <span className="badge b-ok">{"● Telegram"}</span>
              </div>
              {"\n  "}
              <div className="small" style={{ fontSize: "12px" }}>
                {"平台原价 · 零加价"}
              </div>
              {"\n  "}
              <div className="foot">
                <span>
                  <svg className="i" viewBox="0 0 24 24">
                    <rect x="3" y="11" width="18" height="11" rx="2"></rect>
                    <path d="M7 11V7a5 5 0 0 1 10 0v4"></path>
                  </svg>
                  {"禁止：下单交易"}
                </span>
                <span className="more">
                  {"查看 "}
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M5 12h14M12 5l7 7-7 7"></path>
                  </svg>
                </span>
              </div>
            </VisualLink>
            <VisualLink
              className="card sol "
              href={s.siteHref("/solutions/news-aggregator")}
              data-category="research"
              data-channels="Telegram"
              hidden={!s.solutionVisible("research", "Telegram")}
              effect="card"
            >
              {"\n  "}
              <div className="top">
                <div className="ic">
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M4 22h16a2 2 0 0 0 2-2V4a2 2 0 0 0-2-2H8a2 2 0 0 0-2 2v16a2 2 0 0 1-2 2Zm0 0a2 2 0 0 1-2-2v-9c0-1.1.9-2 2-2h2"></path>
                    <path d="M18 14h-8"></path>
                    <path d="M15 18h-5"></path>
                    <path d="M10 6h8v4h-8V6Z"></path>
                  </svg>
                </div>
                <span className="badge b-m">{"研究与洞察"}</span>
              </div>
              {"\n  "}
              <h3>{"新闻聚合"}</h3>
              <p>{"每天早上阅读你关注的信息源，去重后发你一份清爽的简报。"}</p>
              {"\n  "}
              <div className="tags">
                <span className="badge b-ok">{"● Telegram"}</span>
              </div>
              {"\n  "}
              <div className="small" style={{ fontSize: "12px" }}>
                {"平台原价 · 零加价"}
              </div>
              {"\n  "}
              <div className="foot">
                <span>
                  <svg className="i" viewBox="0 0 24 24">
                    <rect x="3" y="11" width="18" height="11" rx="2"></rect>
                    <path d="M7 11V7a5 5 0 0 1 10 0v4"></path>
                  </svg>
                  {"只读"}
                </span>
                <span className="more">
                  {"查看 "}
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M5 12h14M12 5l7 7-7 7"></path>
                  </svg>
                </span>
              </div>
            </VisualLink>
            <VisualLink
              className="card sol "
              href={s.siteHref("/solutions/desk-researcher")}
              data-category="research"
              data-channels="Telegram"
              hidden={!s.solutionVisible("research", "Telegram")}
              effect="card"
            >
              {"\n  "}
              <div className="top">
                <div className="ic">
                  <svg className="i" viewBox="0 0 24 24">
                    <circle cx="11" cy="11" r="8"></circle>
                    <path d="m21 21-4.3-4.3"></path>
                  </svg>
                </div>
                <span className="badge b-m">{"研究与洞察"}</span>
              </div>
              {"\n  "}
              <h3>{"案头研究员"}</h3>
              <p>{"搜索、阅读、比较，然后把完整报告写进你的 Google 文档。"}</p>
              {"\n  "}
              <div className="tags">
                <span className="badge b-ok">{"● Telegram"}</span>
              </div>
              {"\n  "}
              <div className="small" style={{ fontSize: "12px" }}>
                {"平台原价 · 零加价"}
              </div>
              {"\n  "}
              <div className="foot">
                <span>
                  <svg className="i" viewBox="0 0 24 24">
                    <rect x="3" y="11" width="18" height="11" rx="2"></rect>
                    <path d="M7 11V7a5 5 0 0 1 10 0v4"></path>
                  </svg>
                  {"禁止：删除文件"}
                </span>
                <span className="more">
                  {"查看 "}
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M5 12h14M12 5l7 7-7 7"></path>
                  </svg>
                </span>
              </div>
            </VisualLink>
            <VisualLink
              className="card sol "
              href={s.siteHref("/solutions/x-lead-catcher")}
              data-category="sales"
              data-channels="X"
              hidden={!s.solutionVisible("sales", "X")}
              effect="card"
            >
              {"\n  "}
              <div className="top">
                <div className="ic">
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="m3 11 18-5v12L3 14v-3z"></path>
                    <path d="M11.6 16.8a3 3 0 1 1-5.8-1.6"></path>
                  </svg>
                </div>
                <span className="badge b-m">{"销售与营销"}</span>
              </div>
              {"\n  "}
              <h3>{"X 线索捕手"}</h3>
              <p>{"在 X 私信中回答产品问题，筛选线索并加入你的表格。"}</p>
              {"\n  "}
              <div className="tags">
                <span className="badge b-ok">{"● X"}</span>
              </div>
              {"\n  "}
              <div className="small" style={{ fontSize: "12px" }}>
                {"平台原价 · 零加价"}
              </div>
              {"\n  "}
              <div className="foot">
                <span>
                  <svg className="i" viewBox="0 0 24 24">
                    <rect x="3" y="11" width="18" height="11" rx="2"></rect>
                    <path d="M7 11V7a5 5 0 0 1 10 0v4"></path>
                  </svg>
                  {"禁止：公开发帖"}
                </span>
                <span className="more">
                  {"查看 "}
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M5 12h14M12 5l7 7-7 7"></path>
                  </svg>
                </span>
              </div>
            </VisualLink>
            <VisualLink
              className="card sol "
              href={s.siteHref("/solutions/back-office-clerk")}
              data-category="ops"
              data-channels="Telegram,Computer"
              hidden={!s.solutionVisible("ops", "Telegram,Computer")}
              effect="card"
            >
              {"\n  "}
              <div className="top">
                <div className="ic">
                  <svg className="i" viewBox="0 0 24 24">
                    <rect x="2" y="3" width="20" height="14" rx="2"></rect>
                    <path d="M8 21h8M12 17v4"></path>
                  </svg>
                </div>
                <span className="badge b-m">{"运营"}</span>
              </div>
              {"\n  "}
              <h3>{"后台文员"}</h3>
              <p>
                {
                  "登录没有 API 的供应商和预订后台，补货、下载发票并归档，在你的电脑或托管虚拟机上完成。"
                }
              </p>
              {"\n  "}
              <div className="tags">
                <span className="badge b-ok">{"● Telegram"}</span>
                <span className="badge b-ro">{"● 电脑"}</span>
              </div>
              {"\n  "}
              <div className="small" style={{ fontSize: "12px" }}>
                {"平台原价 · 零加价"}
              </div>
              {"\n  "}
              <div className="foot">
                <span>
                  <svg className="i" viewBox="0 0 24 24">
                    <rect x="3" y="11" width="18" height="11" rx="2"></rect>
                    <path d="M7 11V7a5 5 0 0 1 10 0v4"></path>
                  </svg>
                  {"禁止：运行命令"}
                </span>
                <span className="more">
                  {"查看 "}
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M5 12h14M12 5l7 7-7 7"></path>
                  </svg>
                </span>
              </div>
            </VisualLink>
            <VisualLink
              className="card sol"
              href={s.siteHref("/nyxbot")}
              style={{
                borderStyle: "dashed",
                alignItems: "center",
                justifyContent: "center",
                textAlign: "center",
              }}
              effect="card"
            >
              <div className="ic">
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M12 3l1.9 5.8L20 11l-6.1 2.2L12 19l-1.9-5.8L4 11l6.1-2.2z"></path>
                </svg>
              </div>
              <h3>{"描述你自己的需求"}</h3>
              <p>{"“有人询价时，收集他们的地址并发邮件给我。”"}</p>
              <span className="btn sm" style={{ marginTop: "6px" }}>
                {"交给 NyxBot"}
              </span>
            </VisualLink>
          </div>
          {"\n  "}
        </div>
        {"\n"}
      </section>
      {"\n"}
      <section className="s glowbg">
        <div className="wrap split">
          {"\n  "}
          <div>
            <div className="over v">{"企业方案"}</div>
            <h2 className="h2" style={{ marginTop: "10px" }}>
              {"规模化运行？"}
              <span className="grad">{"定制专属套餐。"}</span>
            </h2>
            <p className="lead" style={{ marginTop: "14px" }}>
              {
                "每个方案都按平台原价运行、零加价。如果团队需要可预期的支出、更大用量或特定部署方式，我们会按你的业务定制套餐。"
              }
            </p>
            <div
              style={{
                display: "flex",
                gap: "10px",
                flexWrap: "wrap",
                marginTop: "24px",
              }}
            >
              <VisualLink
                className="btn p"
                href={s.authHref("register")}
                effect="button"
              >
                {"开始使用"}
              </VisualLink>
              <a className="btn" href={s.siteHref("/contact?topic=bundle")}>
                {"联系我们"}
              </a>
            </div>
          </div>
          {"\n  "}
          <div className="card" style={{ padding: "26px" }}>
            {"\n    "}
            <div className="over">{"套餐可以包含"}</div>
            {"\n    "}
            <ul className="checks" style={{ marginTop: "18px", gap: "12px" }}>
              <li>
                <span className="ck">
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M20 6 9 17l-5-5"></path>
                  </svg>
                </span>
                <div>
                  <b style={{ fontSize: "14.5px", fontWeight: "500" }}>
                    {"覆盖所有方案的可预期月度支出"}
                  </b>
                </div>
              </li>
              <li>
                <span className="ck">
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M20 6 9 17l-5-5"></path>
                  </svg>
                </span>
                <div>
                  <b style={{ fontSize: "14.5px", fontWeight: "500" }}>
                    {"面向高流量渠道和智能体的用量"}
                  </b>
                </div>
              </li>
              <li>
                <span className="ck">
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M20 6 9 17l-5-5"></path>
                  </svg>
                </span>
                <div>
                  <b style={{ fontSize: "14.5px", fontWeight: "500" }}>
                    {"SSO、组织角色和管理员管控"}
                  </b>
                </div>
              </li>
              <li>
                <span className="ck">
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M20 6 9 17l-5-5"></path>
                  </svg>
                </span>
                <div>
                  <b style={{ fontSize: "14.5px", fontWeight: "500" }}>
                    {"本地凭据节点或自托管部署"}
                  </b>
                </div>
              </li>
              <li>
                <span className="ck">
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M20 6 9 17l-5-5"></path>
                  </svg>
                </span>
                <div>
                  <b style={{ fontSize: "14.5px", fontWeight: "500" }}>
                    {"优先支持与上线协助"}
                  </b>
                </div>
              </li>
            </ul>
            {"\n  "}
          </div>
          {"\n"}
        </div>
      </section>
      {"\n\n"}
      <section className="s">
        <div className="wrap" style={{ maxWidth: "900px" }}>
          {"\n  "}
          <div className="center" style={{ textAlign: "center" }}>
            <div className="over v">{"方案的一天"}</div>
            <h2
              className="h2"
              style={{ marginTop: "10px", marginBottom: "36px" }}
            >
              {"在你忙自己的事情时"}
            </h2>
          </div>
          {"\n  "}
          <div className="tl">
            <div className="t">
              <div className="card">
                <span className="time">{"08:00"}</span>
                <div>
                  <b>{"发送你的早报"}</b>
                  <div className="small">{"新闻聚合"}</div>
                </div>
                <span className="badge b-ok">{"已完成"}</span>
              </div>
            </div>
            <div className="t">
              <div className="card">
                <span className="time">{"11:20"}</span>
                <div>
                  <b>{"回答“有停车位吗？”"}</b>
                  <div className="small">{"24 小时客服"}</div>
                </div>
                <span className="badge b-ok">{"已完成"}</span>
              </div>
            </div>
            <div className="t">
              <div className="card">
                <span className="time">{"14:10"}</span>
                <div>
                  <b>{"在供应商后台补货"}</b>
                  <div className="small">{"后台文员"}</div>
                </div>
                <span className="badge b-ask">{"待你确认"}</span>
              </div>
            </div>
            <div className="t">
              <div className="card">
                <span className="time">{"15:05"}</span>
                <div>
                  <b>{"回电 6 位客户：3 位已预约"}</b>
                  <div className="small">{"回访队列"}</div>
                </div>
                <span className="badge b-ok">{"已完成"}</span>
              </div>
            </div>
            <div className="t">
              <div className="card">
                <span className="time">{"23:42"}</span>
                <div>
                  <b>{"打烊后订好一张 4 人桌"}</b>
                  <div className="small">{"餐厅经理"}</div>
                </div>
                <span className="badge b-ok">{"已完成"}</span>
              </div>
            </div>
          </div>
          {"\n"}
        </div>
      </section>
      {"\n\n"}
      <section className="s">
        {"\n  "}
        <div className="wrap">
          {"\n    "}
          <div className="over v">{"按行业"}</div>
          <h2
            className="h2"
            style={{ marginTop: "10px", marginBottom: "32px" }}
          >
            {"热门组合"}
          </h2>
          {"\n    "}
          <div className="grid g4">
            {"\n      "}
            <div className="card">
              <div className="ic">
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M3 2v7c0 1.1.9 2 2 2h4a2 2 0 0 0 2-2V2"></path>
                  <path d="M7 2v20"></path>
                  <path d="M21 15V2a5 5 0 0 0-5 5v6c0 1.1.9 2 2 2h3Zm0 0v7"></path>
                </svg>
              </div>
              <h3 style={{ fontSize: "var(--fs-h3)", margin: "14px 0 10px" }}>
                {"餐厅与咖啡馆"}
              </h3>
              <a
                className="more"
                style={{ display: "flex", padding: "4px 0" }}
                href={s.siteHref("/solutions/restaurant-manager")}
              >
                {"餐厅经理 "}
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M5 12h14M12 5l7 7-7 7"></path>
                </svg>
              </a>
              <a
                className="more"
                style={{ display: "flex", padding: "4px 0" }}
                href={s.siteHref("/solutions/back-office-clerk")}
              >
                {"后台文员 "}
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M5 12h14M12 5l7 7-7 7"></path>
                </svg>
              </a>
              <a
                className="more"
                style={{ display: "flex", padding: "4px 0" }}
                href={s.siteHref("/solutions/callback-queue")}
              >
                {"回访队列 "}
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M5 12h14M12 5l7 7-7 7"></path>
                </svg>
              </a>
            </div>
            <div className="card">
              <div className="ic">
                <svg className="i" viewBox="0 0 24 24">
                  <circle cx="6" cy="6" r="3"></circle>
                  <circle cx="6" cy="18" r="3"></circle>
                  <path d="M20 4 8.12 15.88M14.47 14.48 20 20M8.12 8.12 12 12"></path>
                </svg>
              </div>
              <h3 style={{ fontSize: "var(--fs-h3)", margin: "14px 0 10px" }}>
                {"美发与诊所"}
              </h3>
              <a
                className="more"
                style={{ display: "flex", padding: "4px 0" }}
                href={s.siteHref("/solutions/appointment-booker")}
              >
                {"预约助手 "}
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M5 12h14M12 5l7 7-7 7"></path>
                </svg>
              </a>
              <a
                className="more"
                style={{ display: "flex", padding: "4px 0" }}
                href={s.siteHref("/solutions/callback-queue")}
              >
                {"回访队列 "}
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M5 12h14M12 5l7 7-7 7"></path>
                </svg>
              </a>
              <a
                className="more"
                style={{ display: "flex", padding: "4px 0" }}
                href={s.siteHref("/solutions/help-desk")}
              >
                {"24 小时客服 "}
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M5 12h14M12 5l7 7-7 7"></path>
                </svg>
              </a>
            </div>
            <div className="card">
              <div className="ic">
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M21 15a2 2 0 0 1-2 2H7l-4 4V5a2 2 0 0 1 2-2h14a2 2 0 0 1 2 2z"></path>
                </svg>
              </div>
              <h3 style={{ fontSize: "var(--fs-h3)", margin: "14px 0 10px" }}>
                {"网店"}
              </h3>
              <a
                className="more"
                style={{ display: "flex", padding: "4px 0" }}
                href={s.siteHref("/solutions/help-desk")}
              >
                {"24 小时客服 "}
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M5 12h14M12 5l7 7-7 7"></path>
                </svg>
              </a>
              <a
                className="more"
                style={{ display: "flex", padding: "4px 0" }}
                href={s.siteHref("/solutions/x-lead-catcher")}
              >
                {"X 线索捕手 "}
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M5 12h14M12 5l7 7-7 7"></path>
                </svg>
              </a>
              <a
                className="more"
                style={{ display: "flex", padding: "4px 0" }}
                href={s.siteHref("/solutions/back-office-clerk")}
              >
                {"后台文员 "}
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M5 12h14M12 5l7 7-7 7"></path>
                </svg>
              </a>
            </div>
            <div className="card">
              <div className="ic">
                <svg className="i" viewBox="0 0 24 24">
                  <polyline points="22 7 13.5 15.5 8.5 10.5 2 17"></polyline>
                  <polyline points="16 7 22 7 22 13"></polyline>
                </svg>
              </div>
              <h3 style={{ fontSize: "var(--fs-h3)", margin: "14px 0 10px" }}>
                {"创业者与分析师"}
              </h3>
              <a
                className="more"
                style={{ display: "flex", padding: "4px 0" }}
                href={s.siteHref("/solutions/stock-analyst")}
              >
                {"股票分析师 "}
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M5 12h14M12 5l7 7-7 7"></path>
                </svg>
              </a>
              <a
                className="more"
                style={{ display: "flex", padding: "4px 0" }}
                href={s.siteHref("/solutions/news-aggregator")}
              >
                {"新闻聚合 "}
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M5 12h14M12 5l7 7-7 7"></path>
                </svg>
              </a>
              <a
                className="more"
                style={{ display: "flex", padding: "4px 0" }}
                href={s.siteHref("/solutions/desk-researcher")}
              >
                {"案头研究员 "}
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M5 12h14M12 5l7 7-7 7"></path>
                </svg>
              </a>
            </div>
            {"\n    "}
          </div>
          {"\n  "}
        </div>
        {"\n"}
      </section>
      {"\n"}
      <section className="s">
        <div className="wrap">
          <div className="band">
            <h2 className="h2">{"没找到你的工作？"}</h2>
            <p style={{ opacity: ".88", marginTop: "12px", fontSize: "17px" }}>
              {"用一句话描述，NyxBot 会为你定制一个助手。"}
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
