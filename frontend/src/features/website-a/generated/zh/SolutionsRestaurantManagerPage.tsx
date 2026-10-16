// Generated from approved A, SHA-256 5ba81989df18ad10986c1f00cebdcb1e2efd294932853a7671685187a7e34d33.
// Run: node scripts/website-a/generate.mjs <baseline.html>
import { VisualLink } from "../../visuals/elements";
import { useWebsite } from "../../website-context";
export default function SolutionsRestaurantManagerPage() {
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
              <span>{"餐饮与服务"}</span>
            </div>
            {"\n      "}
            <div className="ic lg g">
              <svg className="i" viewBox="0 0 24 24">
                <path d="M3 2v7c0 1.1.9 2 2 2h4a2 2 0 0 0 2-2V2"></path>
                <path d="M7 2v20"></path>
                <path d="M21 15V2a5 5 0 0 0-5 5v6c0 1.1.9 2 2 2h3Zm0 0v7"></path>
              </svg>
            </div>
            {"\n      "}
            <h1
              className="h1"
              style={{ fontSize: "var(--fs-h1)", marginTop: "20px" }}
            >
              {"餐厅经理"}
            </h1>
            {"\n      "}
            <p className="lead" style={{ margin: "16px 0 0" }}>
              {"在 Telegram 上接受订位，查看你的日历，记录忌口并发送提醒。"}
            </p>
            {"\n      "}
            <p
              style={{ marginTop: "14px", fontWeight: "600", fontSize: "17px" }}
            >
              {"再也不会错过深夜订位。"}
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
              <VisualLink
                className="btn p lg"
                href={s.authHref("register")}
                effect="button"
              >
                {"使用此方案"}
              </VisualLink>
              <a className="btn lg" href={s.siteHref("/how-it-works")}>
                {"工作原理"}
              </a>
            </div>
            {"\n      "}
            <div className="pills" style={{ marginTop: "20px" }}>
              <span className="badge b-ok">{"● Telegram"}</span>
              <span className="badge b-m">{"Telegram"}</span>
              <span className="badge b-m">{"Google 日历"}</span>
              <span className="badge b-m">{"Google 表格"}</span>
              <span className="badge b-m">{"支付"}</span>
            </div>
            {"\n    "}
          </div>
          {"\n    "}
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
                <b style={{ fontSize: "14px" }}>{"餐厅经理"}</b>
                <div className="small" style={{ fontSize: "11.5px" }}>
                  {"Telegram · 助手"}
                </div>
              </div>
            </div>
            <div className="msgs">
              <div className="m sys">{"周六 晚上 11:42 · 餐厅已打烊"}</div>
              <div className="m in">
                {"你好，我是 Sarah！明晚 7:30 有 4 人位吗？我们有一位吃素。"}
              </div>
              <div className="m tool">{"✓ 已查日历 · 7:30 有空"}</div>
              <div className="m tool">{"✓ 已订位 · 已添加备注"}</div>
              <div className="m out">
                {
                  "订好了！周日晚上 7:30，4 人桌，Sarah 名下。已备注素食，厨房会知道。"
                }
              </div>
              <div className="m in">{"太好了，谢谢！"}</div>
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
              <b>{"客人给你的机器人发消息"}</b>
              {"Telegram，随时"}
            </div>
            <div className="fn ">
              <div className="k">{"助手"}</div>
              <b>{"理解需求"}</b>
              {"日期、时间、人数、备注"}
            </div>
            <div className="fn ">
              <div className="k">{"执行"}</div>
              <b>{"查询并订位"}</b>
              {"Google 日历 + 客人表格"}
            </div>
            <div className="fn gate">
              <div className="k">{"需你确认"}</div>
              <b>{"仅限取消"}</b>
              {"手机上一键确认"}
            </div>
            <div className="fn ">
              <div className="k">{"结果"}</div>
              <b>{"客人收到确认"}</b>
              {"前一天发送提醒"}
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
                <b>{"餐厅经理"}</b>
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
              <span className="gl" style={{ background: "#229ED9" }}>
                {"TG"}
              </span>
              <div>
                {"Telegram"}
                <small>{"回复客人"}</small>
              </div>
              <span className="badge b-ok">{"允许"}</span>
            </div>
            <div className="prow">
              <span className="gl" style={{ background: "#4285F4" }}>
                {"31"}
              </span>
              <div>
                {"Google 日历"}
                <small>{"查空位并订桌"}</small>
              </div>
              <span className="badge b-ok">{"允许"}</span>
            </div>
            <div className="prow">
              <span className="gl" style={{ background: "#0F9D58" }}>
                {"SH"}
              </span>
              <div>
                {"Google 表格"}
                <small>{"客人备注与过敏信息"}</small>
              </div>
              <span className="badge b-ok">{"允许"}</span>
            </div>
            <div className="prow">
              <span className="gl" style={{ background: "#4285F4" }}>
                {"31"}
              </span>
              <div>
                {"Google 日历"}
                <small>{"取消订位"}</small>
              </div>
              <span className="badge b-ask">{"先问我"}</span>
            </div>
            <div className="prow">
              <span className="gl" style={{ background: "#52525B" }}>
                {"$"}
              </span>
              <div>
                {"支付"}
                <small>{"收取订金"}</small>
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
                  <b>{"营业时间和桌位大小"}</b>
                </div>
              </li>
              <li>
                <span className="ck">{"2"}</span>
                <div>
                  <b>{"订位使用的日历"}</b>
                </div>
              </li>
              <li>
                <span className="ck">{"3"}</span>
                <div>
                  <b>{"菜单或过敏说明（可选）"}</b>
                </div>
              </li>
              <li>
                <span className="ck">{"4"}</span>
                <div>
                  <b>{"回复的语气和语言"}</b>
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
                  "此方案按各服务的收费计费：Telegram、Google 日历、Google 表格以及 AI 模型，零加价。早期体验期间网络费用由我们承担。"
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
                  "规模化运行餐厅经理？我们会根据你的用量、部署和支持需求定制套餐。"
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
            <summary>{"如果订满了怎么办？"}</summary>
            <p>
              {"它会推荐最近的空闲时段，也可以把客人加入表格中的候补名单。"}
            </p>
          </details>
          <details>
            <summary>{"客人可以取消吗？"}</summary>
            <p>
              {
                "可以。但默认情况下，取消需要你先一键确认，你也可以更改这一设置。"
              }
            </p>
          </details>
          <details>
            <summary>{"能和我的订位系统配合吗？"}</summary>
            <p>
              {
                "开箱即用支持 Google 日历和表格。提供 API 的系统可以直接接入，没有 API 的系统也可以通过连接的电脑来操作。"
              }
            </p>
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
            <VisualLink
              className="card sol "
              href={s.siteHref("/solutions/appointment-booker")}
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
              href={s.siteHref("/solutions/stock-analyst")}
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
          </div>
        </div>
      </section>
      {"\n"}
      <section className="s">
        <div className="wrap">
          <div className="band">
            <h2 className="h2">{"试用餐厅经理"}</h2>
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
