// Generated from approved A, SHA-256 5ba81989df18ad10986c1f00cebdcb1e2efd294932853a7671685187a7e34d33.
// Run: node scripts/website-a/generate.mjs <baseline.html>
import { SiteLink } from "../../site-link";
import { useWebsite } from "../../website-context";
export default function SecurityPage() {
  const s = useWebsite();
  void s;
  return (
    <>
      {"\n"}
      <section className="hero glowbg" style={{ position: "relative" }}>
        {"\n  "}
        <div className="wrap">
          <div className="ic lg" style={{ margin: "0 auto" }}>
            <svg className="i" viewBox="0 0 24 24">
              <path d="M12 22s8-4 8-10V5l-8-3-8 3v7c0 6 8 10 8 10z"></path>
            </svg>
          </div>
          <h1
            className="h1"
            style={{ marginTop: "22px", fontSize: "var(--fs-h1)" }}
          >
            {"你的密码，"}
            <br />
            <span className="grad">{"永远不会交给 AI。"}</span>
          </h1>
          <p className="lead">
            {
              "登录信息加密保存在 NyxID 中。每个助手只拿到你允许操作的限权密钥，高风险操作等你确认，每次调用都有记录。"
            }
          </p>
        </div>
        {"\n"}
      </section>
      {"\n\n"}
      <section className="s">
        <div className="wrap">
          <div className="grid g3">
            {"\n  "}
            <div className="card">
              <div className="ic">
                <svg className="i" viewBox="0 0 24 24">
                  <circle cx="7.5" cy="15.5" r="5.5"></circle>
                  <path d="m21 2-9.6 9.6M15.5 7.5l3 3L22 7l-3-3"></path>
                </svg>
              </div>
              <h3 style={{ fontSize: "var(--fs-h3)", margin: "14px 0 6px" }}>
                {"限权密钥"}
              </h3>
              <p className="sub" style={{ fontSize: "14.5px" }}>
                {"每个助手只获得工作所需的操作权限。"}
              </p>
            </div>
            <div className="card">
              <div className="ic">
                <svg className="i" viewBox="0 0 24 24">
                  <rect x="3" y="11" width="18" height="11" rx="2"></rect>
                  <path d="M7 11V7a5 5 0 0 1 10 0v4"></path>
                </svg>
              </div>
              <h3 style={{ fontSize: "var(--fs-h3)", margin: "14px 0 6px" }}>
                {"密码存于保险库"}
              </h3>
              <p className="sub" style={{ fontSize: "14.5px" }}>
                {"登录信息加密存储，绝不展示给 AI。"}
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
                {"人工审批"}
              </h3>
              <p className="sub" style={{ fontSize: "14.5px" }}>
                {"高风险操作等你点“同意”。"}
              </p>
            </div>
            <div className="card">
              <div className="ic">
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M8 6h13M8 12h13M8 18h13M3 6h.01M3 12h.01M3 18h.01"></path>
                </svg>
              </div>
              <h3 style={{ fontSize: "var(--fs-h3)", margin: "14px 0 6px" }}>
                {"防篡改日志"}
              </h3>
              <p className="sub" style={{ fontSize: "14.5px" }}>
                {"每个操作都记录在哈希链日志中。"}
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
                {"电脑访问需主动开启"}
              </h3>
              <p className="sub" style={{ fontSize: "14.5px" }}>
                {"浏览器、文件、桌面和命令行在你逐项开启前都处于关闭状态。"}
              </p>
            </div>
            <div className="card">
              <div className="ic">
                <svg className="i" viewBox="0 0 24 24">
                  <path d="M18.36 6.64a9 9 0 1 1-12.73 0"></path>
                  <path d="M12 2v10"></path>
                </svg>
              </div>
              <h3 style={{ fontSize: "var(--fs-h3)", margin: "14px 0 6px" }}>
                {"一键关停"}
              </h3>
              <p className="sub" style={{ fontSize: "14.5px" }}>
                {"一键暂停助手或撤销密钥。"}
              </p>
            </div>
            {"\n"}
          </div>
        </div>
      </section>
      {"\n\n"}
      <section className="s">
        <div className="wrap split">
          {"\n  "}
          <div>
            <div className="over v">{"1 · 限权密钥"}</div>
            <h2 className="h2" style={{ marginTop: "10px" }}>
              {"只给工作所需的权限"}
            </h2>
            <p className="lead" style={{ marginTop: "14px" }}>
              {
                "大多数 AI 工具会要求账号的全部权限。NyxID 把权限拆分到每个操作：“查看空闲时间”和“创建预约”与“取消预约”或“发送邮件”是分开的。"
              }
            </p>
            {"\n  "}
            <ul className="checks" style={{ marginTop: "24px" }}>
              <li>
                <span className="ck">
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M20 6 9 17l-5-5"></path>
                  </svg>
                </span>
                <div>
                  <b>{"每个助手一把密钥"}</b>
                  <span>{"一个助手的权限不会泄露给另一个。"}</span>
                </div>
              </li>
              <li>
                <span className="ck">
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M20 6 9 17l-5-5"></path>
                  </svg>
                </span>
                <div>
                  <b>{"细节层面的限制"}</b>
                  <span>{"例如：“只拨打我名单上的号码”。"}</span>
                </div>
              </li>
              <li>
                <span className="ck">
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M20 6 9 17l-5-5"></path>
                  </svg>
                </span>
                <div>
                  <b>{"频率限制"}</b>
                  <span>{"按助手设置的频率限制可防止失控循环。"}</span>
                </div>
              </li>
            </ul>
          </div>
          {"\n  "}
          <div className="perm-card">
            <div className="pc-h">
              <div>
                <b>{"回访队列"}</b>
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
              <span className="gl" style={{ background: "#0F9D58" }}>
                {"SH"}
              </span>
              <div>
                {"Google 表格"}
                <small>{"读取通话名单"}</small>
              </div>
              <span className="badge b-ro">{"只读"}</span>
            </div>
            <div className="prow">
              <span className="gl" style={{ background: "#0F9D58" }}>
                {"SH"}
              </span>
              <div>
                {"Google 表格"}
                <small>{"写入结果"}</small>
              </div>
              <span className="badge b-ok">{"允许"}</span>
            </div>
            <div className="prow">
              <span className="gl" style={{ background: "#F22F46" }}>
                {"TW"}
              </span>
              <div>
                {"Twilio"}
                <small>{"拨打名单上的号码"}</small>
              </div>
              <span className="badge b-ok">{"允许"}</span>
            </div>
            <div className="prow">
              <span className="gl" style={{ background: "#18181B" }}>
                {"11"}
              </span>
              <div>
                {"ElevenLabs"}
                <small>{"自然语音"}</small>
              </div>
              <span className="badge b-ok">{"允许"}</span>
            </div>
            <div className="prow">
              <span className="gl" style={{ background: "#52525B" }}>
                {"$"}
              </span>
              <div>
                {"折扣与退款"}
                <small>{"提供退款或优惠"}</small>
              </div>
              <span className="badge b-ask">{"先问我"}</span>
            </div>
          </div>
          {"\n"}
        </div>
      </section>
      {"\n\n"}
      <section className="s">
        <div className="wrap split rev">
          {"\n  "}
          <div>
            <div className="over v">{"2 · 密码保险库"}</div>
            <h2 className="h2" style={{ marginTop: "10px" }}>
              {"你的登录信息永远不会交给 AI"}
            </h2>
            <p className="lead" style={{ marginTop: "14px" }}>
              {
                "你在服务自己的页面上登录。NyxID 以信封加密保存凭据，并在每次请求的最后一刻才注入，因此它永远不会出现在提示词、聊天记录或 AI 服务商的日志中。"
              }
            </p>
          </div>
          {"\n  "}
          <div className="card" style={{ padding: "28px" }}>
            {"\n    "}
            <div
              className="grid vault"
              style={{
                gridTemplateColumns: "1fr auto 1fr auto 1fr",
                alignItems: "center",
                gap: "10px",
                textAlign: "center",
                fontSize: "13px",
              }}
            >
              {"\n      "}
              <div>
                <div className="ic" style={{ margin: "0 auto" }}>
                  <svg className="i" viewBox="0 0 24 24">
                    <circle cx="12" cy="8" r="4"></circle>
                    <path d="M4 21v-1a6 6 0 0 1 6-6h4a6 6 0 0 1 6 6v1"></path>
                  </svg>
                </div>
                <b style={{ display: "block", marginTop: "8px" }}>{"你"}</b>
                <span className="small">{"登录 Google"}</span>
              </div>
              <span className="small">{"→"}</span>
              {"\n      "}
              <div>
                <div className="ic g lg" style={{ margin: "0 auto" }}>
                  <svg className="i" viewBox="0 0 24 24">
                    <rect x="3" y="11" width="18" height="11" rx="2"></rect>
                    <path d="M7 11V7a5 5 0 0 1 10 0v4"></path>
                  </svg>
                </div>
                <b style={{ display: "block", marginTop: "8px" }}>
                  {"NyxID 保险库"}
                </b>
                <span className="small">{"AES-256 加密"}</span>
              </div>
              <span className="small">{"→"}</span>
              {"\n      "}
              <div>
                <div className="ic" style={{ margin: "0 auto" }}>
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M12 3l1.9 5.8L20 11l-6.1 2.2L12 19l-1.9-5.8L4 11l6.1-2.2z"></path>
                  </svg>
                </div>
                <b style={{ display: "block", marginTop: "8px" }}>
                  {"你的助手"}
                </b>
                <span className="small">{"获得限权密钥"}</span>
              </div>
              {"\n    "}
            </div>
            {"\n    "}
            <div className="ph" style={{ marginTop: "22px" }}>
              {"AI 看到的是： "}
              <code style={{ margin: "0 6px" }}>{"nyxid_ag_••••"}</code>
              {" · 永远不是你的密码"}
            </div>
            {"\n  "}
          </div>
          {"\n"}
        </div>
      </section>
      {"\n\n"}
      <section className="s">
        <div className="wrap">
          {"\n  "}
          <div style={{ maxWidth: "620px" }}>
            <div className="over v">{"3 · 人工审批"}</div>
            <h2 className="h2" style={{ marginTop: "10px" }}>
              {"由你决定它问多少"}
            </h2>
            <p className="lead" style={{ marginTop: "14px" }}>
              {"为助手选择访问模式。可以统一设置，也可以为每个助手单独设置。"}
            </p>
          </div>
          {"\n  "}
          <div className="grid g3" style={{ marginTop: "32px" }}>
            {"\n    "}
            <div className="card">
              <span className="badge b-m">{"自动驾驶"}</span>
              <h3 style={{ fontSize: "var(--fs-h3)", margin: "14px 0 6px" }}>
                {"直接执行"}
              </h3>
              <p className="sub" style={{ fontSize: "14.5px" }}>
                {"不发审批卡片。最适合新闻简报这类只读工作。"}
              </p>
            </div>
            {"\n    "}
            <div className="card feat">
              <span className="badge b-v">{"NyxMode · 默认"}</span>
              <h3 style={{ fontSize: "var(--fs-h3)", margin: "14px 0 6px" }}>
                {"大事才问"}
              </h3>
              <p className="sub" style={{ fontSize: "14.5px" }}>
                {"在删除、取消、付款等敏感或破坏性操作前先问你。"}
              </p>
            </div>
            {"\n    "}
            <div className="card">
              <span className="badge b-ask">{"每次都问"}</span>
              <h3 style={{ fontSize: "var(--fs-h3)", margin: "14px 0 6px" }}>
                {"全部确认"}
              </h3>
              <p className="sub" style={{ fontSize: "14.5px" }}>
                {"任何更改前都会询问。适合建立信任的初期。"}
              </p>
            </div>
            {"\n  "}
          </div>
          {"\n"}
        </div>
      </section>
      {"\n\n"}
      <section className="s">
        <div className="wrap split">
          {"\n  "}
          <div>
            <div className="over v">{"4 · 电脑访问"}</div>
            <h2 className="h2" style={{ marginTop: "10px" }}>
              {"你的机器，你来开关"}
            </h2>
            <p className="lead" style={{ marginTop: "14px" }}>
              {
                "NyxBot 使用电脑时，每项功能默认关闭，并在机器本地控制。只有你（所有者）能使用；与你的机器人聊天的客户永远不能。"
              }
            </p>
            {"\n  "}
            <ul className="checks" style={{ marginTop: "24px" }}>
              <li>
                <span className="ck">
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M20 6 9 17l-5-5"></path>
                  </svg>
                </span>
                <div>
                  <b>{"只接受签名请求"}</b>
                  <span>{"电脑只接受为它签名的请求。"}</span>
                </div>
              </li>
              <li>
                <span className="ck">
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M20 6 9 17l-5-5"></path>
                  </svg>
                </span>
                <div>
                  <b>{"隐藏的已保存登录"}</b>
                  <span>{"只填入指定网站，绝不展示给 AI。"}</span>
                </div>
              </li>
              <li>
                <span className="ck">
                  <svg className="i" viewBox="0 0 24 24">
                    <path d="M20 6 9 17l-5-5"></path>
                  </svg>
                </span>
                <div>
                  <b>{"可选更强隔离"}</b>
                  <span>
                    {"使用 NyxID 容器或托管虚拟机，让工作与个人文件分开。"}
                  </span>
                </div>
              </li>
            </ul>
          </div>
          {"\n  "}
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
          {"\n"}
        </div>
      </section>
      {"\n\n"}
      <section className="s">
        <div className="wrap split rev">
          {"\n  "}
          <div>
            <div className="over v">{"5 · 操作记录"}</div>
            <h2 className="h2" style={{ marginTop: "10px" }}>
              {"值得信赖的记录"}
            </h2>
            <p className="lead" style={{ marginTop: "14px" }}>
              {
                "每个操作都写入审计日志，每条记录都与前一条链接，任何修改或删除都会暴露。你始终知道哪个助手在何时做了什么。"
              }
            </p>
          </div>
          {"\n  "}
          <div className="card log">
            {"\n    "}
            <div className="r">
              <span className="tm">{"23:42:08"}</span>
              <span>{"餐厅经理 · calendar.create_event"}</span>
              <span className="badge b-ok">{"允许"}</span>
            </div>
            <div className="r">
              <span className="tm">{"19:40:31"}</span>
              <span>{"餐厅经理 · calendar.delete_event"}</span>
              <span className="badge b-ask">{"已由你批准"}</span>
            </div>
            <div className="r">
              <span className="tm">{"14:10:02"}</span>
              <span>{"后台文员 · machine.browser"}</span>
              <span className="badge b-ask">{"已由你批准"}</span>
            </div>
            <div className="r">
              <span className="tm">{"15:05:12"}</span>
              <span>{"回访队列 · twilio.create_call"}</span>
              <span className="badge b-ok">{"允许"}</span>
            </div>
            <div className="r">
              <span className="tm">{"15:05:44"}</span>
              <span>{"回访队列 · twilio.create_call (+44…)"}</span>
              <span className="badge b-no">{"已拦截：不在名单中"}</span>
            </div>
            {"\n  "}
          </div>
          {"\n"}
        </div>
      </section>
      {"\n\n"}
      <section className="s">
        <div className="wrap split">
          {"\n  "}
          <div>
            <div className="over v">{"6 · 关停开关"}</div>
            <h2 className="h2" style={{ marginTop: "10px" }}>
              {"随时叫停一切"}
            </h2>
            <p className="lead" style={{ marginTop: "14px" }}>
              {
                "一键停用助手、撤销密钥或断开服务。正在运行的任务会停止，AI 立即失去访问权限。"
              }
            </p>
          </div>
          {"\n  "}
          <div className="card mock" style={{ display: "grid", gap: "12px" }}>
            <div
              style={{
                display: "flex",
                justifyContent: "space-between",
                alignItems: "center",
                padding: "6px 0",
              }}
            >
              <b style={{ fontSize: "14.5px" }}>{"餐厅经理"}</b>
              <span
                style={{ display: "flex", gap: "10px", alignItems: "center" }}
              >
                <span className="badge b-ok">{"运行中"}</span>
                <button className="btn sm" type="button">
                  {"停用"}
                </button>
              </span>
            </div>
            <div
              style={{
                display: "flex",
                justifyContent: "space-between",
                alignItems: "center",
                padding: "6px 0",
              }}
            >
              <b style={{ fontSize: "14.5px" }}>{"后台文员"}</b>
              <span
                style={{ display: "flex", gap: "10px", alignItems: "center" }}
              >
                <span className="badge b-ok">{"运行中"}</span>
                <button className="btn sm" type="button">
                  {"停用"}
                </button>
              </span>
            </div>
            <div
              style={{
                display: "flex",
                justifyContent: "space-between",
                alignItems: "center",
                padding: "6px 0",
              }}
            >
              <b style={{ fontSize: "14.5px" }}>{"新闻聚合"}</b>
              <span
                style={{ display: "flex", gap: "10px", alignItems: "center" }}
              >
                <span className="badge b-m">{"已暂停"}</span>
                <button className="btn sm" type="button">
                  {"启用"}
                </button>
              </span>
            </div>
          </div>
          {"\n"}
        </div>
      </section>
      {"\n\n"}
      <section className="s">
        <div className="wrap">
          {"\n  "}
          <div className="over v">{"给你的 IT 团队"}</div>
          <h2 className="h2" style={{ margin: "10px 0 32px" }}>
            {"技术细节"}
          </h2>
          {"\n  "}
          <div className="grid g3">
            <div className="card">
              <b style={{ fontSize: "15px" }}>{"加密"}</b>
              <p
                className="small"
                style={{ marginTop: "8px", fontSize: "14px" }}
              >
                {
                  "AES-256 信封加密；支持 AWS KMS 和 GCP Cloud KMS；密钥材料保存在自动清零的内存中。"
                }
              </p>
            </div>
            <div className="card">
              <b style={{ fontSize: "15px" }}>{"身份认证"}</b>
              <p
                className="small"
                style={{ marginTop: "8px", fontSize: "14px" }}
              >
                {
                  "OAuth 2.0 / OpenID Connect + PKCE、多因素认证、单点登录、短期令牌。"
                }
              </p>
            </div>
            <div className="card">
              <b style={{ fontSize: "15px" }}>{"智能体隔离"}</b>
              <p
                className="small"
                style={{ marginTop: "8px", fontSize: "14px" }}
              >
                {
                  "每个智能体独立密钥，带服务白名单、操作范围、输入限制和频率限制。"
                }
              </p>
            </div>
            <div className="card">
              <b style={{ fontSize: "15px" }}>{"审计"}</b>
              <p
                className="small"
                style={{ marginTop: "8px", fontSize: "14px" }}
              >
                {"HMAC 哈希链审计日志，可在管理后台的完整性页面验证。"}
              </p>
            </div>
            <div className="card">
              <b style={{ fontSize: "15px" }}>{"机器节点"}</b>
              <p
                className="small"
                style={{ marginTop: "8px", fontSize: "14px" }}
              >
                {
                  "本地掌控的命令行/文件/电脑开关、强制签名请求、仅限所有者使用、人工实时桌面。"
                }
              </p>
            </div>
            <div className="card">
              <b style={{ fontSize: "15px" }}>{"本地部署选项"}</b>
              <p
                className="small"
                style={{ marginTop: "8px", fontSize: "14px" }}
              >
                {"凭据节点把密钥保存在你自己的机器上，NyxID 通过它们转发请求。"}
              </p>
            </div>
          </div>
          {"\n  "}
          <div
            className="card"
            style={{
              marginTop: "16px",
              display: "flex",
              justifyContent: "space-between",
              alignItems: "center",
              gap: "20px",
              flexWrap: "wrap",
            }}
          >
            <div style={{ maxWidth: "640px" }}>
              <b style={{ fontSize: "var(--fs-h3)" }}>{"开源，Apache-2.0"}</b>
              <p
                className="sub"
                style={{ fontSize: "14.5px", marginTop: "6px" }}
              >
                {
                  "本页的每一项说法都可以核实。阅读代码、用 Docker 自行部署，或让你自己的 AI 审查密钥如何存储、解密和限权。"
                }
              </p>
            </div>
            <SiteLink
              className="btn"
              href="https://github.com/ChronoAIProject/NyxID"
              target="_blank"
              rel="noopener noreferrer"
            >
              <svg
                className="dc"
                width="16"
                height="16"
                viewBox="0 0 24 24"
                aria-hidden="true"
              >
                <path d="M12 .5C5.65.5.5 5.65.5 12a11.5 11.5 0 0 0 7.86 10.92c.58.1.79-.25.79-.56v-2c-3.2.7-3.88-1.37-3.88-1.37-.52-1.33-1.28-1.69-1.28-1.69-1.04-.71.08-.7.08-.7 1.16.08 1.77 1.19 1.77 1.19 1.03 1.76 2.7 1.25 3.36.96.1-.75.4-1.25.73-1.54-2.56-.29-5.25-1.28-5.25-5.68 0-1.26.45-2.28 1.19-3.09-.12-.29-.52-1.46.11-3.05 0 0 .97-.31 3.17 1.18a11 11 0 0 1 5.77 0c2.2-1.49 3.17-1.18 3.17-1.18.63 1.59.23 2.76.11 3.05.74.81 1.19 1.83 1.19 3.09 0 4.41-2.69 5.38-5.26 5.67.41.36.78 1.06.78 2.14v3.17c0 .31.21.67.8.56A11.5 11.5 0 0 0 23.5 12C23.5 5.65 18.35.5 12 .5Z"></path>
              </svg>
              {" 在 GitHub 查看"}
            </SiteLink>
          </div>
          {"\n"}
        </div>
      </section>
      {"\n\n"}
      <section className="s">
        <div className="wrap" style={{ maxWidth: "860px" }}>
          <div className="over v">{"常见问题"}</div>
          <h2 className="h2" style={{ margin: "10px 0 18px" }}>
            {"安全问题"}
          </h2>
          <details>
            <summary>{"NyxID 员工能看到我的密码吗？"}</summary>
            <p>
              {
                "登录信息加密存储，只有在执行被允许的请求时才会解密，绝不会出现在控制台或 API 响应中。"
              }
            </p>
          </details>
          <details>
            <summary>{"如果 AI 被客户消息诱导了怎么办？"}</summary>
            <p>
              {
                "它仍然只能做密钥允许的事。客户永远无法触及你的电脑，任何敏感操作都需要你批准，被拦截的尝试会出现在日志中。"
              }
            </p>
          </details>
          <details>
            <summary>{"让 AI 使用我的电脑有风险吗？"}</summary>
            <p>
              {
                "任何电脑访问都有风险，所以每项功能默认关闭，每次开启都需要你亲自操作。若要最强隔离，请使用容器或托管虚拟机，而不是你的日常电脑。"
              }
            </p>
          </details>
          <details>
            <summary>{"我的数据存储在哪里？"}</summary>
            <p>{"请联系团队确认当前的部署与认证信息。"}</p>
          </details>
          <details>
            <summary>{"你们有安全认证吗？"}</summary>
            <p>{"请联系团队确认当前的部署与认证信息。"}</p>
          </details>
        </div>
      </section>
      {"\n"}
      <section className="s">
        <div className="wrap">
          <div className="band">
            <h2 className="h2">{"值得信赖的自动化。"}</h2>
            <p style={{ opacity: ".88", marginTop: "12px", fontSize: "17px" }}>
              {"从只读方案开始，逐步增加更多。"}
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
