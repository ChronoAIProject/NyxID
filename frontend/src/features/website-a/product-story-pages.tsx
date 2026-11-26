import { FileCheck2, KeyRound } from "lucide-react";
import { useWebsite } from "./website-context";
import { CodeExample } from "./interactions";
import { AppPill, PermissionReceipt, StoryDetails, StoryHero, StoryLink, StoryNext, StoryPage, StorySection, StorySplit } from "./story-page";
import { NyxbotAppShowcase, NyxbotApprovalCapture, NyxbotMemoryCapture } from "./nyxbot-app-media";
import "./product-story-pages.css";

export function HowItWorksStoryPage() {
  const s = useWebsite(), zh = s.language === "zh";
  const steps = zh ? [
    ["在聊天里交代任务", "你在 Telegram、X 或 NyxID 应用里发消息。请求交给 NyxBot，也可以由你现有的智能体发起。", "帮 Sarah 订明晚 7:30 的四人桌。"],
    ["智能体选择工具", "日历用来查空位和创建预订，表格用来保存客人信息。它调用的是你已经连接的应用。", "Google 日历 · Google 表格"],
    ["NyxID 检查权限", "每次请求都要通过当前的服务与操作权限。需要审批的步骤等待确认，被禁止的操作无法执行。", "创建预订：允许。取消预订：先确认。"],
    ["发出请求时添加登录凭据", "NyxID 为允许的请求加入对应账号的凭据，再发给服务商。真实密码与 API 密钥不会进入智能体的对话。", "智能体 → NyxID → 你连接的账号"],
    ["应用完成操作", "预订写入日历，客人信息存入表格，确认消息发回原来的聊天。", "Sarah 的四人桌，晚上 7:30。"],
    ["留下一条可查的记录", "活动日志记录哪个密钥发起请求、调用了哪个服务，以及结果。之后可以检查记录或修改权限。", "调用者、服务、时间、结果。"],
  ] : [
    ["A request arrives in your chat", "Send a message on Telegram, X or the NyxID app. NyxBot handles the request, or your existing agent can make the calls.", "Book Sarah a table for four tomorrow at 7:30."],
    ["The agent chooses the tools", "Calendar checks availability and creates the booking. Sheets keeps the guest details. These are the apps you connected.", "Google Calendar and Google Sheets"],
    ["NyxID checks the permissions", "Each call must pass the current service and operation limits. An action that needs approval waits. A blocked action cannot proceed.", "Create booking: allowed. Cancel booking: approval required."],
    ["The login is added before the call", "NyxID adds the account credential to an allowed request and sends it to the provider. The real password or API key stays out of the conversation.", "Agent → NyxID → your connected account"],
    ["The apps do the work", "The booking goes into Calendar, guest details go into Sheets, and the confirmation returns to the original chat.", "Sarah’s table for four, 7:30 PM."],
    ["The call leaves a record", "Activity shows which key called which service and the result. You can review the record or change access when the job changes.", "Caller, service, time and result."],
  ];
  return <StoryPage className="st-product st-how">
    <StoryHero title={zh ? "一次请求，如何变成完成的工作" : "How a request becomes work"} description={zh ? "连接应用和聊天渠道，让智能体在你设定的权限内做事。下面跟随一次预订请求，看看每一步发生了什么。" : "Connect your apps and chat channels, then let an agent act within the access you give it. Follow a booking request through the tools, checks and result."} />
    <section className="st-journey" aria-label={zh ? "一次预订的完整流程，示例" : "A booking request from start to finish, example"}>
      <div className="st-wrap st-journey-layout"><div className="st-journey-context"><p>{zh ? "示例" : "Example"}</p><h2>{zh ? "预订一张四人桌" : "A table for four"}</h2><div className="st-apps"><AppPill name="Telegram" icon="telegram" /><AppPill name="Calendar" icon="google-calendar" /><AppPill name="Sheets" icon="google-sheets" /></div><StoryLink href={s.siteHref("/nyxbot")}>NyxBot</StoryLink></div>
      <div><h2 id="request-sequence" className="sr-only">{zh ? "从请求到结果" : "From request to result"}</h2><ol className="st-journey-steps" aria-labelledby="request-sequence">{steps.map(([title, body, detail], index) => <li key={title}><span className="st-journey-number">{index + 1}</span><div><h3>{title}</h3><p>{body}</p><div className="st-journey-detail">{detail}</div></div></li>)}</ol></div></div>
    </section>
    <StoryNext title={zh ? "先连接这份工作需要的工具" : "Connect the tools for your first task"} description={zh ? "查看可用服务及它们的连接方式。" : "Find the services you use and their connection requirements."} href={s.siteHref("/integrations")} label={zh ? "工具" : "Tools"} />
  </StoryPage>;
}

export function NyxbotStoryPage() {
  const s = useWebsite(), zh = s.language === "zh";
  return <StoryPage className="st-product st-nyxbot">
    <StoryHero title={zh ? "让 NyxBot 在你的应用里做事" : "NyxBot works with your apps"} description={zh ? "在 Telegram、X 或 NyxID 应用里交代预订、简报或回访任务。NyxBot 使用你连接的工具，也能把长期工作交给专属助手。" : "Ask for a booking, a brief or a callback on Telegram, X or the NyxID app. NyxBot uses your connected tools and can delegate recurring work to specialists."}>
      <StoryLink primary href={s.authHref("register")}>{zh ? "开始使用 NyxBot" : "Start with NyxBot"}</StoryLink>
    </StoryHero>
    <NyxbotAppShowcase />
    <StorySection title={zh ? "在熟悉的聊天里继续" : "Use the chat you already have"} description={zh ? "消息、结果和需要确认的事项，都回到同一段对话里。" : "Messages, results and requests for approval come back to the same conversation."}>
      <div className="st-nyxbot-channels"><div><h3>{zh ? "从这些渠道发消息" : "Send the request from"}</h3><div className="st-apps"><AppPill name="Telegram" icon="telegram" /><AppPill name="X" icon="x" /><AppPill name={zh ? "NyxID 应用" : "NyxID app"} /><AppPill name="WhatsApp" icon="whatsapp" soon /></div></div><div><h3>{zh ? "连接任务需要的应用" : "Connect the apps it needs"}</h3><div className="st-apps"><AppPill name="Gmail" icon="gmail" /><AppPill name="Calendar" icon="google-calendar" /><AppPill name="Sheets" icon="google-sheets" /><AppPill name="Notion" icon="notion" /></div><StoryLink href={s.siteHref("/integrations")}>{zh ? "工具" : "Tools"}</StoryLink></div></div>
    </StorySection>
    <StorySection><StorySplit><div><h2>{zh ? "专属助手记住每份工作的细节" : "Specialists keep the details for each job"}</h2><p className="st-lead">{zh ? "预订助手记住无障碍餐桌和简报时间。研究助手保留自己的偏好与对话。每个助手使用获准的工具，并把结果汇报给 NyxBot。" : "A booking specialist remembers the accessible table and the time to send a summary. A researcher keeps its own notes and conversations. Each uses the tools it has been granted and reports back to NyxBot."}</p><p className="st-nyxbot-security-note">{zh ? "访问权限和需要审批的操作可以分别设置。" : "You choose each specialist’s access and which actions need approval."}</p><StoryLink href={s.siteHref("/security")}>{zh ? "安全" : "Security"}</StoryLink></div><NyxbotMemoryCapture /></StorySplit></StorySection>
    <StoryNext title={zh ? "看看可以交给 NyxBot 的工作" : "Find a workflow for NyxBot"} description={zh ? "从一个预订、简报或后台工作场景开始。" : "Browse examples for bookings, research, calls and office work."} href={s.siteHref("/solutions")} label={zh ? "解决方案" : "Solutions"} />
  </StoryPage>;
}

export function SecurityStoryPage() {
  const s = useWebsite(), zh = s.language === "zh";
  return <StoryPage className="st-product st-security">
    <StoryHero title={zh ? "凭据由 NyxID 保管" : "Keep credentials out of the agent"} description={zh ? "智能体使用 NyxID 身份发起请求。真实凭据保留在 NyxID 或你的凭据节点中；服务、操作和审批规则决定它能做什么。" : "The agent makes calls with a NyxID identity. Real credentials stay in NyxID or your credential node. Service access, operation limits and approval rules determine what it can do."} visual={<PermissionReceipt />} />
    <StorySection id="approvals"><StorySplit><div><h2>{zh ? "一次确认，只授权这次操作" : "Approval applies to one action"}</h2><p className="st-lead">{zh ? "你可以允许创建预订，同时让取消操作等待确认。确认卡会说明具体请求；批准后才会继续执行。" : "You can allow bookings while requiring approval for cancellations. A confirmation card identifies the action; the agent waits for a decision before continuing."}</p><StoryLink href={s.siteHref("/docs/ai/guides/agent-isolation")}>{zh ? "权限与密钥指南" : "Permissions and keys guide"}</StoryLink></div><NyxbotApprovalCapture /></StorySplit></StorySection>
    <StorySection><StorySplit align="start"><div><h2>{zh ? "凭据在哪里使用" : "Where the credential is used"}</h2><p className="st-lead">{zh ? "你在服务商页面登录，或把 API 密钥连接到 NyxID。允许的请求发出时，代理层才加入真实凭据。模型不需要拿到它。" : "Sign in with the provider or connect an API key to NyxID. The proxy adds the real credential when an allowed request goes out. The model does not need to receive it."}</p></div><div><StoryDetails summary={zh ? "加密与身份" : "Encryption and identity"}><p>{zh ? "AES-256 信封加密，支持 AWS KMS 和 GCP Cloud KMS。身份层支持 OAuth 2.0、OpenID Connect、PKCE、MFA 和 SSO。" : "AES-256 envelope encryption with AWS KMS and GCP Cloud KMS support. Identity includes OAuth 2.0, OpenID Connect, PKCE, MFA and SSO."}</p></StoryDetails><StoryDetails summary={zh ? "自己的凭据节点" : "Your own credential node"}><p>{zh ? "凭据节点把密钥留在你自己的机器上，NyxID 把请求路由到相应节点。浏览器、文件、桌面与终端访问需要分别开启。" : "Credential nodes keep secrets on your own machines, with NyxID routing requests through the node. Browser, file, desktop and shell access must be enabled individually."}</p></StoryDetails><StoryDetails summary={zh ? "托管地点与认证" : "Hosting and certifications"}><p>{zh ? "联系团队，确认当前的托管地点和认证情况。" : "Contact the team to confirm current hosting and certification details."}</p><StoryLink href={s.siteHref("/contact")}>{zh ? "联系" : "Contact"}</StoryLink></StoryDetails></div></StorySplit></StorySection>
    <StorySection title={zh ? "检查记录，修改后续访问" : "Review calls and change future access"}><div className="st-security-records"><div><FileCheck2 size={21} /><h3>{zh ? "审计记录" : "Audit records"}</h3><p>{zh ? "活动日志标明发起请求的密钥、服务和结果。新记录使用 HMAC-SHA256 哈希链，管理员可以在 Integrity 页面验证记录完整性。" : "Activity identifies the key, service and result. New audit records use an HMAC-SHA256 hash chain; administrators can verify chain integrity in the Integrity page."}</p></div><div><KeyRound size={21} /><h3>{zh ? "暂停与撤销" : "Pause or revoke access"}</h3><p>{zh ? "工作改变时，可以暂停助手、撤销密钥或断开服务，控制之后的访问。Apache-2.0 开源实现可以检查，也可以自托管。" : "Pause an assistant, revoke its key or disconnect a service when the job changes. The Apache-2.0 source is available to inspect or self-host."}</p><StoryLink href="https://github.com/ChronoAIProject/NyxID">{zh ? "源代码" : "Source code"}</StoryLink></div></div></StorySection>
    <StoryNext title={zh ? "了解服务与平台费用" : "Understand the provider and platform charges"} href={s.siteHref("/pricing")} label={zh ? "价格" : "Pricing"} />
  </StoryPage>;
}

const agentCode = [
  `# Print the configuration for your MCP client\nnyxid mcp config --tool claude-code\nnyxid mcp config --tool cursor\nnyxid mcp config --tool codex\n\n# Run the printed command and sign in through your browser.`,
  `# Use the slug of a service you connected\ncurl https://nyx-api.chrono-ai.fun/api/v1/proxy/s/llm-openai/models \\\n  -H "Authorization: Bearer $NYXID_API_KEY"`,
  `# Choose a model available through your connected account\ncurl https://nyx-api.chrono-ai.fun/api/v1/llm/gateway/v1/chat/completions \\\n  -H "Authorization: Bearer $NYXID_API_KEY" \\\n  -H "Content-Type: application/json" \\\n  -d '{"model":"YOUR_CONNECTED_MODEL","messages":[{"role":"user","content":"Summarise today’s bookings"}]}'`,
  `nyxid api-key create --name "booking-agent" --platform generic --scopes "proxy"\n\n# Use the key ID and service UUIDs returned by NyxID\nnyxid api-key update "$NYXID_KEY_ID" \\\n  --allowed-services "$CALENDAR_SERVICE_ID" \\\n  --allow-all-services false`,
];

export function AgentsStoryPage() {
  const s = useWebsite(), zh = s.language === "zh";
  const setup = zh ? [["连接服务商账号", "把智能体需要的账号添加到 NyxID。真正的服务商凭据保留在这里。"], ["连接客户端", "桌面 MCP 客户端通过浏览器登录。自己的代码或无人值守进程可以使用智能体密钥。"], ["检查访问范围", "新建密钥默认可访问所有服务。使用前选择所需服务，并关闭“允许所有服务”。"]] : [["Connect a provider account", "Add the accounts your agent needs to NyxID. Their real credentials stay here."], ["Connect the client", "Desktop MCP clients sign in through a browser. Your own code or an unattended process can use an Agent Key."], ["Review the service scope", "New keys allow all services by default. Select the services this agent needs and turn off Allow all services before using it."]];
  return <StoryPage className="st-product st-agents">
    <StoryHero title={zh ? "连接你已经在用的智能体" : "Connect the agent you already use"} description={zh ? "Claude、Cursor、Codex 和自己的代码，都能通过 NyxID 调用已连接的服务。使用 MCP、REST 或兼容 OpenAI 的模型接口。" : "Claude, Cursor, Codex and your own code can call connected services through NyxID. Use MCP, REST or the OpenAI-compatible model gateway."}>
      <div className="st-apps"><AppPill name="Claude" icon="claude" /><AppPill name="Cursor" icon="cursor" /><AppPill name="Codex" icon="openai" /></div>
    </StoryHero>
    <section className="st-agent-connect" id="connection-examples" aria-label={zh ? "连接代码示例" : "Connection examples"}><div className="st-wrap"><CodeExample labels={["MCP", "REST", "Models", "CLI"]} snippets={agentCode} /></div></section>
    <StorySection title={zh ? "第一次调用前" : "Before the first call"}><ol className="st-agent-setup">{setup.map(([title, text], index) => <li key={title}><span>{index + 1}</span><h3>{title}</h3><p>{text}</p></li>)}</ol></StorySection>
    <StorySection><div className="st-agent-guides"><div><h2>{zh ? "客户端与授权方式" : "Clients and authentication"}</h2><p>{zh ? "MCP 指南涵盖浏览器登录和工具发现；密钥指南说明独立服务范围、凭据绑定与调用速率。" : "The MCP guide covers browser sign-in and tool discovery. The key guide covers separate service scopes, credential bindings and rate limits."}</p></div><div><StoryLink href={s.siteHref("/docs/ai/guides/mcp-proxy")}>{zh ? "MCP 设置指南" : "MCP setup guide"}</StoryLink><StoryLink href={s.siteHref("/docs/ai/guides/agent-isolation")}>{zh ? "智能体密钥指南" : "Agent Key guide"}</StoryLink><StoryLink href="https://github.com/ChronoAIProject/NyxID">{zh ? "源代码" : "Source code"}</StoryLink></div></div></StorySection>
    <StoryNext title={zh ? "了解 NyxID 如何检查访问" : "See how NyxID checks access"} description={zh ? "凭据、审批、审计与撤销。" : "Credentials, approvals, audit records and revocation."} href={s.siteHref("/security")} label={zh ? "安全" : "Security"} />
  </StoryPage>;
}
