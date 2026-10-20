// Generated from approved A, SHA-256 5ba81989df18ad10986c1f00cebdcb1e2efd294932853a7671685187a7e34d33.
// Run: node scripts/website-a/generate.mjs <baseline.html>
import { SiteLink } from "../../site-link";
import { VisualLink } from "../../visuals/elements";
import { SolutionActivation } from "../../solution-activation";
import { useWebsite } from "../../website-context";
export default function SolutionsXLeadCatcherPage() {
  const s = useWebsite();
  void s;
  return (
    <>
      {"\n"}
      <section
        className="hero glowbg"
        style={{
          textAlign: "left",
          paddingBottom: "64px",
          position: "relative",
        }}
      >
        {"\n  "}
        <div className="wrap split">
          {"\n    "}
          <div>
            {"\n      "}
            <div className="crumbs">
              <SiteLink href={s.siteHref("/solutions")}>{"方案"}</SiteLink>
              <span>{"/"}</span>
              <span>{"销售与营销"}</span>
            </div>
            {"\n      "}
            <div className="ic lg g">
              <svg className="i" viewBox="0 0 24 24">
                <path d="m3 11 18-5v12L3 14v-3z"></path>
                <path d="M11.6 16.8a3 3 0 1 1-5.8-1.6"></path>
              </svg>
            </div>
            {"\n      "}
            <h1
              className="h1"
              style={{ fontSize: "var(--fs-h1)", marginTop: "20px" }}
            >
              {"X 线索捕手"}
            </h1>
            {"\n      "}
            <p className="lead" style={{ margin: "16px 0 0" }}>
              {"在 X 私信中回答产品问题，筛选线索并加入你的表格。"}
            </p>
            {"\n      "}
            <p
              style={{ marginTop: "14px", fontWeight: "600", fontSize: "17px" }}
            >
              {"不再有已读不回的私信。"}
            </p>
            {"\n      "}
            <div
              style={{
                display: "flex",
                gap: "10px",
                flexWrap: "wrap",
                marginTop: "28px",
              }}
            >
              <SolutionActivation />
              <SiteLink className="btn lg" href={s.siteHref("/how-it-works")}>
                {"工作原理"}
              </SiteLink>
            </div>
            {"\n      "}
            <div className="pills" style={{ marginTop: "20px" }}>
              <span className="badge b-ok">{"● X"}</span>
              <span className="badge b-m">{"X 私信"}</span>
              <span className="badge b-m">{"Google 表格"}</span>
              <span className="badge b-m">{"X（推特）"}</span>
            </div>
            {"\n    "}
          </div>
          {"\n    "}
          <div className="phone">
            <div className="ph-h">
              <div className="av">
                <svg className="i" viewBox="0 0 24 24">
                  <path d="m3 11 18-5v12L3 14v-3z"></path>
                  <path d="M11.6 16.8a3 3 0 1 1-5.8-1.6"></path>
                </svg>
              </div>
              <div>
                <b style={{ fontSize: "14px" }}>{"X 线索捕手"}</b>
                <div className="small" style={{ fontSize: "11.5px" }}>
                  {"X · 助手"}
                </div>
              </div>
            </div>
            <div className="msgs">
              <div className="m in">{"你好，你们的应用适合 20 人团队吗？"}</div>
              <div className="m out">
                {"适合！团队版 10 个席位起。要不要帮你约个演示？"}
              </div>
              <div className="m in">{"好，下周吧"}</div>
              <div className="m tool">{"✓ 已添加线索 · 已申请演示"}</div>
            </div>
          </div>
          {"\n  "}
        </div>
        {"\n"}
      </section>
      {"\n\n"}
      <section className="s">
        <div className="wrap">
          {"\n  "}
          <div className="over v">{"运行方式"}</div>
          <h2 className="h2" style={{ margin: "10px 0 32px" }}>
            {"从收到消息到完成"}
          </h2>
          {"\n  "}
          <div className="flow">
            <div className="fn ">
              <div className="k">{"触发"}</div>
              <b>{"新私信"}</b>
              {"X"}
            </div>
            <div className="fn ">
              <div className="k">{"助手"}</div>
              <b>{"回答并筛选"}</b>
              {"规模、需求、时间"}
            </div>
            <div className="fn ">
              <div className="k">{"执行"}</div>
              <b>{"记录线索"}</b>
              {"Google 表格"}
            </div>
            <div className="fn gate">
              <div className="k">{"需你确认"}</div>
              <b>{"不需要"}</b>
              {"它不能公开发帖"}
            </div>
            <div className="fn ">
              <div className="k">{"结果"}</div>
              <b>{"表格中的优质线索"}</b>
              {"随时跟进"}
            </div>
          </div>
          {"\n"}
        </div>
      </section>
      {"\n"}
      <section className="s">
        <div className="wrap split">
          {"\n  "}
          <div>
            <div className="over v">{"默认安全"}</div>
            <h2 className="h2" style={{ marginTop: "10px" }}>
              {"它被允许做什么"}
            </h2>
            <p className="lead" style={{ marginTop: "14px" }}>
              {"这些权限已经预设好。上线前可以修改，之后也能一键调整。"}
            </p>
            {"\n  "}
            <div
              style={{
                display: "flex",
                gap: "8px",
                flexWrap: "wrap",
                marginTop: "22px",
              }}
            >
              <span className="badge b-ok">{"允许"}</span>
              <span className="badge b-ro">{"只读"}</span>
              <span className="badge b-ask">{"先问我"}</span>
              <span className="badge b-no">{"禁止"}</span>
            </div>
            {"\n  "}
            <SiteLink
              className="more"
              style={{ marginTop: "22px" }}
              href={s.siteHref("/security")}
            >
              {"了解权限如何运作 "}
              <svg className="i" viewBox="0 0 24 24">
                <path d="M5 12h14M12 5l7 7-7 7"></path>
              </svg>
            </SiteLink>
          </div>
          {"\n  "}
          <div className="perm-card">
            <div className="pc-h">
              <div>
                <b>{"X 线索捕手"}</b>
                <div className="small">{"这个助手能做什么"}</div>
              </div>
              <span className="badge b-v">
                <svg className="i" viewBox="0 0 24 24">
                  <circle cx="7.5" cy="15.5" r="5.5"></circle>
                  <path d="m21 2-9.6 9.6M15.5 7.5l3 3L22 7l-3-3"></path>
                </svg>
                {" 限权密钥"}
              </span>
            </div>
            <div className="prow">
              <span className="gl" style={{ background: "#27272A" }}>
                {"X"}
              </span>
              <div>
                {"X 私信"}
                <small>{"回复消息"}</small>
              </div>
              <span className="badge b-ok">{"允许"}</span>
            </div>
            <div className="prow">
              <span className="gl" style={{ background: "#0F9D58" }}>
                {"SH"}
              </span>
              <div>
                {"Google 表格"}
                <small>{"添加线索"}</small>
              </div>
              <span className="badge b-ok">{"允许"}</span>
            </div>
            <div className="prow">
              <span className="gl" style={{ background: "#27272A" }}>
                {"X"}
              </span>
              <div>
                {"X（推特）"}
                <small>{"公开发帖"}</small>
              </div>
              <span className="badge b-no">{"禁止"}</span>
            </div>
            <div className="small" style={{ marginTop: "12px" }}>
              {"底层原理：带有操作范围和审批模式的 NyxID Agent Key。"}
            </div>
          </div>
          {"\n"}
        </div>
      </section>
      {"\n"}
      <section className="s">
        <div className="wrap split">
          {"\n  "}
          <div>
            <div className="over v">{"设置"}</div>
            <h2 className="h2" style={{ marginTop: "10px" }}>
              {"你只需填写几项信息"}
            </h2>
            <p className="lead" style={{ marginTop: "14px" }}>
              {"用大白话回答即可。之后在聊天中告诉 NyxBot 就能随时修改。"}
            </p>
          </div>
          {"\n  "}
          <div className="card">
            <ul className="checks">
              <li>
                <span className="ck">{"1"}</span>
                <div>
                  <b>{"产品信息和价格"}</b>
                </div>
              </li>
              <li>
                <span className="ck">{"2"}</span>
                <div>
                  <b>{"用哪些问题筛选线索"}</b>
                </div>
              </li>
              <li>
                <span className="ck">{"3"}</span>
                <div>
                  <b>{"你的线索表格"}</b>
                </div>
              </li>
            </ul>
          </div>
          {"\n"}
        </div>
      </section>
      {"\n"}
      <section className="s">
        <div className="wrap">
          <div className="over v">{"价格"}</div>
          <h2 className="h2" style={{ margin: "10px 0 28px" }}>
            {"费用说明"}
          </h2>
          {"\n  "}
          <div className="grid g2">
            {"\n    "}
            <div className="card">
              <span className="badge b-ok">{"● 用量"}</span>
              <h3 style={{ fontSize: "var(--fs-h3)", margin: "14px 0 6px" }}>
                {"平台原价"}
              </h3>
              <p className="sub" style={{ fontSize: "14.5px" }}>
                {
                  "此方案按各服务的收费计费：X 私信、Google 表格以及 AI 模型，零加价。早期体验期间网络费用由我们承担。"
                }
              </p>
              <SiteLink
                className="more"
                style={{ marginTop: "12px" }}
                href={s.siteHref("/pricing")}
              >
                {"价格说明 "}
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M5 12h14M12 5l7 7-7 7"></path>
                </svg>
              </SiteLink>
            </div>
            {"\n    "}
            <div className="card">
              <span className="badge b-m">{"企业"}</span>
              <h3 style={{ fontSize: "var(--fs-h3)", margin: "14px 0 6px" }}>
                {"定制套餐"}
              </h3>
              <p className="sub" style={{ fontSize: "14.5px" }}>
                {
                  "规模化运行X 线索捕手？我们会根据你的用量、部署和支持需求定制套餐。"
                }
              </p>
              <SiteLink
                className="more"
                style={{ marginTop: "12px" }}
                href={s.siteHref("/contact?topic=bundle")}
              >
                {"联系我们 "}
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M5 12h14M12 5l7 7-7 7"></path>
                </svg>
              </SiteLink>
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
            {"关于此方案"}
          </h2>
          <details>
            <summary>{"它能替我发帖吗？"}</summary>
            <p>{"不能。此方案中关闭了公开发帖。"}</p>
          </details>
        </div>
      </section>
      {"\n"}
      <section className="s">
        <div className="wrap">
          <div className="sechead">
            <h2 className="h2">{"相关方案"}</h2>
            <SiteLink className="btn" href={s.siteHref("/solutions")}>
              {"全部方案 "}
              <svg className="i" viewBox="0 0 24 24">
                <path d="M5 12h14M12 5l7 7-7 7"></path>
              </svg>
            </SiteLink>
          </div>
          <div className="grid g3">
            <VisualLink
              className="card sol "
              href={s.siteHref("/solutions/help-desk")}
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
          </div>
        </div>
      </section>
      {"\n"}
      <section className="s">
        <div className="wrap">
          <div className="band">
            <h2 className="h2">{"试用X 线索捕手"}</h2>
            <p style={{ opacity: ".88", marginTop: "12px", fontSize: "17px" }}>
              {"几分钟完成设置，随时可以关闭。"}
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
