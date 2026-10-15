// Generated from approved A, SHA-256 5ba81989df18ad10986c1f00cebdcb1e2efd294932853a7671685187a7e34d33.
// Run: node scripts/website-a/generate.mjs <baseline.html>
import { useWebsite } from "../../website-context";
export default function SolutionsDeskResearcherPage() {
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
              <a href={s.siteHref("/solutions")}>{"方案"}</a>
              <span>{"/"}</span>
              <span>{"研究与洞察"}</span>
            </div>
            {"\n      "}
            <div className="ic lg g">
              <svg className="i" viewBox="0 0 24 24">
                <circle cx="11" cy="11" r="8"></circle>
                <path d="m21 21-4.3-4.3"></path>
              </svg>
            </div>
            {"\n      "}
            <h1
              className="h1"
              style={{ fontSize: "var(--fs-h1)", marginTop: "20px" }}
            >
              {"案头研究员"}
            </h1>
            {"\n      "}
            <p className="lead" style={{ margin: "16px 0 0" }}>
              {"搜索、阅读、比较，然后把完整报告写进你的 Google 文档。"}
            </p>
            {"\n      "}
            <p
              style={{ marginTop: "14px", fontWeight: "600", fontSize: "17px" }}
            >
              {"喝杯咖啡的工夫，问题就变成了报告。"}
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
              <a className="btn p lg" href={s.authHref("register")}>
                {"使用此方案"}
              </a>
              <a className="btn lg" href={s.siteHref("/how-it-works")}>
                {"工作原理"}
              </a>
            </div>
            {"\n      "}
            <div className="pills" style={{ marginTop: "20px" }}>
              <span className="badge b-ok">{"● Telegram"}</span>
              <span className="badge b-m">{"Tavily"}</span>
              <span className="badge b-m">{"Firecrawl"}</span>
              <span className="badge b-m">{"Google 文档"}</span>
              <span className="badge b-m">{"Google 云端硬盘"}</span>
            </div>
            {"\n    "}
          </div>
          {"\n    "}
          <div className="phone">
            <div className="ph-h">
              <div className="av">
                <svg className="i" viewBox="0 0 24 24">
                  <circle cx="11" cy="11" r="8"></circle>
                  <path d="m21 21-4.3-4.3"></path>
                </svg>
              </div>
              <div>
                <b style={{ fontSize: "14px" }}>{"案头研究员"}</b>
                <div className="small" style={{ fontSize: "11.5px" }}>
                  {"Telegram · 助手"}
                </div>
              </div>
            </div>
            <div className="msgs">
              <div className="m in">
                {"找 5 个和我们订位应用竞争的产品，比较一下价格。"}
              </div>
              <div className="m tool">{"✓ 已搜索 18 个网页"}</div>
              <div className="m tool">{"✓ 已撰写报告"}</div>
              <div className="m out">
                {
                  "完成！已比较 5 个竞品：价格从免费到每月 79 美元不等。报告在你的 Google 文档“竞品扫描”里。"
                }
              </div>
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
              <b>{"你提出问题"}</b>
              {"Telegram"}
            </div>
            <div className="fn ">
              <div className="k">{"助手"}</div>
              <b>{"规划研究"}</b>
              {"搜什么、读什么"}
            </div>
            <div className="fn ">
              <div className="k">{"执行"}</div>
              <b>{"搜索并阅读"}</b>
              {"Tavily + Firecrawl"}
            </div>
            <div className="fn gate">
              <div className="k">{"需你确认"}</div>
              <b>{"不需要"}</b>
              {"只创建新文档"}
            </div>
            <div className="fn ">
              <div className="k">{"结果"}</div>
              <b>{"完整报告"}</b>
              {"在 Google 文档中"}
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
            <a
              className="more"
              style={{ marginTop: "22px" }}
              href={s.siteHref("/security")}
            >
              {"了解权限如何运作 "}
              <svg className="i" viewBox="0 0 24 24">
                <path d="M5 12h14M12 5l7 7-7 7"></path>
              </svg>
            </a>
          </div>
          {"\n  "}
          <div className="perm-card">
            <div className="pc-h">
              <div>
                <b>{"案头研究员"}</b>
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
              <span className="gl" style={{ background: "#0EA5E9" }}>
                {"TV"}
              </span>
              <div>
                {"Tavily"}
                <small>{"网页搜索"}</small>
              </div>
              <span className="badge b-ro">{"只读"}</span>
            </div>
            <div className="prow">
              <span className="gl" style={{ background: "#3F3F46" }}>
                {"FC"}
              </span>
              <div>
                {"Firecrawl"}
                <small>{"读取网页"}</small>
              </div>
              <span className="badge b-ro">{"只读"}</span>
            </div>
            <div className="prow">
              <span className="gl" style={{ background: "#2563EB" }}>
                {"DC"}
              </span>
              <div>
                {"Google 文档"}
                <small>{"撰写报告"}</small>
              </div>
              <span className="badge b-ok">{"允许"}</span>
            </div>
            <div className="prow">
              <span className="gl" style={{ background: "#16A34A" }}>
                {"DR"}
              </span>
              <div>
                {"Google 云端硬盘"}
                <small>{"删除文件"}</small>
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
                  <b>{"报告保存位置"}</b>
                </div>
              </li>
              <li>
                <span className="ck">{"2"}</span>
                <div>
                  <b>{"偏好格式（表格、备忘录、要点）"}</b>
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
                  "此方案按各服务的收费计费：Tavily、Firecrawl、Google 文档以及 AI 模型，零加价。早期体验期间网络费用由我们承担。"
                }
              </p>
              <a
                className="more"
                style={{ marginTop: "12px" }}
                href={s.siteHref("/pricing")}
              >
                {"价格说明 "}
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M5 12h14M12 5l7 7-7 7"></path>
                </svg>
              </a>
            </div>
            {"\n    "}
            <div className="card">
              <span className="badge b-m">{"企业"}</span>
              <h3 style={{ fontSize: "var(--fs-h3)", margin: "14px 0 6px" }}>
                {"定制套餐"}
              </h3>
              <p className="sub" style={{ fontSize: "14.5px" }}>
                {
                  "规模化运行案头研究员？我们会根据你的用量、部署和支持需求定制套餐。"
                }
              </p>
              <a
                className="more"
                style={{ marginTop: "12px" }}
                href={s.siteHref("/contact?topic=bundle")}
              >
                {"联系我们 "}
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M5 12h14M12 5l7 7-7 7"></path>
                </svg>
              </a>
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
            <summary>{"会注明来源吗？"}</summary>
            <p>{"会，每份报告都列出所用网页。"}</p>
          </details>
          <details>
            <summary>{"能编辑我现有的文档吗？"}</summary>
            <p>{"此方案中不能，它只创建新文档。"}</p>
          </details>
        </div>
      </section>
      {"\n"}
      <section className="s">
        <div className="wrap">
          <div className="sechead">
            <h2 className="h2">{"相关方案"}</h2>
            <a className="btn" href={s.siteHref("/solutions")}>
              {"全部方案 "}
              <svg className="i" viewBox="0 0 24 24">
                <path d="M5 12h14M12 5l7 7-7 7"></path>
              </svg>
            </a>
          </div>
          <div className="grid g3">
            <a
              className="card sol feat"
              href={s.siteHref("/solutions/restaurant-manager")}
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
            </a>
            <a className="card sol " href={s.siteHref("/solutions/help-desk")}>
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
            </a>
            <a
              className="card sol "
              href={s.siteHref("/solutions/appointment-booker")}
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
            </a>
          </div>
        </div>
      </section>
      {"\n"}
      <section className="s">
        <div className="wrap">
          <div className="band">
            <h2 className="h2">{"试用案头研究员"}</h2>
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
