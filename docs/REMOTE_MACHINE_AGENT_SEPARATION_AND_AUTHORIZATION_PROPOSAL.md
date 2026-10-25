# Remote Machine 的 Agent separation 与授权方案

- 日期：2026-10-24。
- 状态：设计建议，尚未实施；本文不代表相关能力已经上线或通过运行时验收。
- 依据：当前仓库的静态代码审阅、非托管机器的既有合同，以及此前本地使用中报告的创建、Specialist 分配、模式确认和 Agent 授权问题。本轮未运行产品测试、Node 或真实 Kubernetes 集群。
- 范围：NyxID 托管 Remote Machine 的 separation 创建、Agent 授权、运行时准备和状态反馈。

## 1. 推荐结论

Remote Machine 与非托管机器采用同一套权限语义，同时由托管平台承担机器端的配置操作：

1. 机器记录后续新增 Agent 的 separation 默认选择。
2. 机器 capabilities 作为 Agent 工具授权的上限。
3. 每个 Agent 分别保存机器分配关系、实际环境模式和工具能力授权。
4. Agent assignment revision 与 Remote Machine machine configuration revision 分开管理；Node 的 Agent authority ACK 不能单独代表托管环境模式已生效。
5. 页面提供完整的 owner 确认与两条应用链的生效进度，不强制用户跳转到 NyxBot 主对话完成模式切换。

推荐采用“完整目标、分阶段交付”：先修复现有页面和状态判断，再打通托管 separation 创建与 Agent 授权。

技术判断为**条件性可行**。阶段一可独立交付；阶段二除了界面和合同贯通，还必须解决托管 Pod 的账号供给、context 持久化、完整 v4 探测及版本兼容。已有本地 shared-mode 镜像验证也不能证明托管 separated 已可用。以下“当前事实”仅陈述源码，“目标合同”描述待实现行为，“验收”均为未来验证要求。

## 2. 使用过程中暴露的问题

本地验证和沟通反映了以下问题：

- 创建非托管机器时可以选择 Agent separation，托管 Remote Machine 没有相同入口。
- 创建时选择了机器能力和 Specialist，用户容易理解为该 Specialist 已经获得这些工具权限。
- Agent 的机器能力复选框不可操作时，用户不容易分辨是机器能力未开启、模式未选择、账号功能未开启，还是节点协议不支持。
- 在机器页面发起模式选择后，还需要到 NyxBot 主对话寻找 Allow 卡片，操作链路不完整。
- 用户需要反复区分“机器默认选择”“Agent 实际模式”“机器能力上限”“Agent 工具授权”。
- 已有机器的能力是否可改、修改是否会重启、何时真正生效，缺少统一说明。

本方案将上述概念和操作入口明确分开，并统一 Remote Machine 与非托管机器的授权行为。

## 3. 当前代码事实

以下属于静态代码审阅结论；运行中的具体账号、节点和集群支持状态需要在实施及验收时重新读取。

| 当前实现 | 影响 | 代码依据 |
|---|---|---|
| 非托管创建页提供 `automatic`、`require_separated`、`shared_legacy` 和完整共享告知 | 可复用选择语义和文案 | [machine-setup.tsx:276](../frontend/src/pages/machine-setup.tsx#L276)、[共享确认:294](../frontend/src/pages/machine-setup.tsx#L294) |
| 托管 `CreateRequest` 有 capabilities、Specialist、共享确认，拒绝未知字段且无 separation；policy 的 `shared_legacy` 是产品告知 | 下拉框需要扩展合同；policy 字段不能当作每个 Agent 的实际模式 | [remote_machine.rs:85](../backend/src/handlers/remote_machine.rs#L85)、[policy:162](../backend/src/handlers/remote_machine.rs#L162) |
| bootstrap `Secrets` 仅有 auth/signing/capabilities；CLI 安装由 capabilities 生成配置，默认 `separation_choice=None` | 需要贯通选择、payload/config 版本与节点报告，不能只增加服务端字段 | [bootstrap:278](../backend/src/services/remote_machine_bootstrap.rs#L278)、[安装:490](../cli/src/node/managed_runtime.rs#L490)、[默认值:49](../machine/src/config.rs#L49) |
| CLI 对整个 machine config 严格比较；manager 验证上报的实际 capability ceiling 与选定值相同 | 数据库写入、安装后本地改配置或普通重启都不能代替受控应用 | [managed_config.rs:59](../cli/src/node/managed_config.rs#L59)、[profile:1934](../backend/src/services/remote_machine_manager.rs#L1934) |
| Access 页不传 `contextEnabled`，默认 false，又用 capability flag 禁用整张表单；Details 分别传两个 flag | 缺参隐藏 separated 入口；shared 入口另有条件，不能概括为所有模式入口均不存在 | [Access:283](../frontend/src/components/machines/remote-machine-agent-access.tsx#L283)、[Access:300](../frontend/src/components/machines/remote-machine-agent-access.tsx#L300)、[Details:31](../frontend/src/components/assistant/machine-capabilities.tsx#L31)、[模式条件:76](../frontend/src/components/assistant/machine-capabilities.tsx#L76) |
| 普通 PUT 拒绝 `mode`；graphical request 已校验真人并服务端生成 typed action、revision、能力和 login 快照，随后仍在 NyxBot thread 确认 | 页面闭环应适配这条授权链，不另造授权权威；首次选择的快照例外见 §6.3 | [PUT:160](../backend/src/handlers/machine_access.rs#L160)、[快照:252](../backend/src/handlers/machine_access.rs#L252)、[typed action:297](../backend/src/handlers/machine_access.rs#L297) |
| Specialist 初始分配复用既有 grants 服务；新 assignment 在 mode 已解析且 capability editor 关闭时复制节点 ceiling，未解析则为空 | “只分配成员、所有工具默认关”是本方案的新托管策略，尚非所有现有 flag 组合的行为 | [分配:95](../backend/src/services/remote_machine_configuration.rs#L95)、[默认授权:105](../backend/src/services/machine_access_service.rs#L105)、[既有回归断言:1823](../backend/src/services/remote_machine_service_tests.rs#L1823) |
| `safe_view` 用 `all()` 要求 Specialist 拥有机器开启的全部能力，否则设 `permissions_required` | 部分有效授权被误判；通过此判断也不证明 mode、在线状态或实际执行可用 | [remote_machine_configuration.rs:146](../backend/src/services/remote_machine_configuration.rs#L146) |
| managed config 根据 Browser 开关建立 dev browser/managed browser；context 创建仍需要这些身份、display 和 browser runtime | Shell/File-only separated 需保留完整基础设施而拒绝浏览器工具，不能仅去掉 support 检查 | [managed_config.rs:30](../cli/src/node/managed_config.rs#L30)、[context_dispatch.rs:232](../cli/src/node/machine/context_dispatch.rs#L232) |
| hosted 镜像预建 1000/1001/1002 三个用户；context 通过 `useradd` 动态供给 `nyxc{uid}`，而 Pod 根文件系统只读、capabilities 固定 | 现有 native 账号创建流程不能直接视为已兼容托管 Pod，见 §7.1 | [镜像:75](../cli/Dockerfile.machine-hosted#L75)、[账号:187](../cli/src/node/machine/context_runtime.rs#L187)、[Pod:277](../machine-updater/src/sandbox.rs#L277)、[admission:35](../deploy/remote-machine/admission.yaml#L35) |
| hosted config 位于 `/var/lib/nyxid-machine/node`；context journal 在该目录，workspace 在其父目录的 `contexts/`；整个 identity 卷为 Memory emptyDir，持久 PVC 只挂 `/workspace` | 当前 separated 路径不能直接满足跨 Pod 的工作区保留，见 §7.2；这是静态路径推导，不是生产丢失事故记录 | [入口:20](../cli/container/hosted-entrypoint.sh#L20)、[journal/root:245](../cli/src/node/machine/context_runtime.rs#L245)、[workspace:358](../cli/src/node/machine/context_runtime.rs#L358)、[卷:289](../machine-updater/src/sandbox.rs#L289) |

这不是单纯的页面字段缺失，而是托管配置、授权入口、运行时准备和状态展示需要共同调整。

## 4. 统一对象与语义

| 设置 | 所属对象 | 修改入口 | 影响范围 |
|---|---|---|---|
| 新 Agent 的 separation 默认选择 | Machine | 创建页、机器设置页 | 后续新增授权 |
| 机器启用的 capabilities | Machine | 创建页、机器设置页 | 所有客户端可获得权限的上限 |
| 是否分配给某个 Agent | Agent 与 Machine 的关系 | Machine → Access、Agent → Details | 该 Agent 是否具有机器成员关系 |
| Agent 实际环境模式 | 单个 Agent 的机器授权 | Machine → Access、Agent → Details | 该 Agent 使用共享环境还是 separated context |
| Agent capabilities | 单个 Agent 的机器授权 | Machine → Access、Agent → Details | 该 Agent 实际可以调用的工具 |

### 4.1 separation 默认选择与实际模式

| 默认选择 | 新授权的处理 |
|---|---|
| `automatic` | 只有完整支持已确认、账号允许创建 context 时才选择 separated；无法确认则等待 owner 处理，不静默切换共享模式 |
| `require_separated` | 必须通过 separated 验证；失败时不能以共享模式继续授权 |
| `shared_legacy` | owner 阅读共享环境说明并明确确认后，记录共享选择 |

实际运行模式仍为 `separated` 或 `shared_legacy`。`automatic` 是选择策略，`selection_required` 是未完成选择的状态，都不是额外的运行模式。

修改默认选择、升级节点或调整 feature flag，不重写已有 Agent 的模式、能力和登录授权。同一台机器可以同时存在共享与 separated 授权。

### 4.2 capabilities 的两层授权

以下为本方案目标；现有 feature-off 兼容分支见 §3，不能据此宣称已经实现：

- 机器开启能力，只提高机器上限，不自动授予任何 Agent 或外部客户端。
- Agent capabilities 可以是机器能力的子集，也可以全部关闭。
- 分配机器成员关系，不等于授予工具能力。
- 实际执行继续检查当前 owner、客户端来源、机器状态、节点能力、Agent 授权、context 和执行权限有效性。
- `separated` 共享机器内核，不表示为每个 Agent 创建了独立虚拟机。
- `shared_legacy` 的 Shell 使用执行 OS 用户的权限；workspace roots 约束文件工具和工作目录，不构成任意 Shell 文件访问的边界。
- capabilities 约束工具 API；Shell 仍可访问其 OS 身份有权访问的文件，Computer 可通过应用 UI 操作。因此关闭 File API 不等于 Shell 无文件能力，关闭某个 API 也不构成更强的 OS 隔离承诺（[既有合同](MACHINE_NODES.md#per-agent-capabilities-m12)）。

### 4.3 权威与责任分工

| 层 | 负责的权威与新增职责 |
|---|---|
| 授权服务 | 保存 Agent assignment / external grant 的实际模式、capabilities、saved logins、授权 revision 和撤销 outbox；复用现有 `configure` 与各来源 fence。Agent assignment revision 只能证明 Agent 授权控制面，不代表托管环境配置已经生效 |
| 托管配置与 runtime 服务 | 保存 separation payload/config 版本、目标 machine configuration revision、runtime generation 和应用观测；控制 bootstrap、版本兼容与托管配置证据收敛 |
| Node/runtime | 对 Agent 授权链证明持久最小 revision、旧 lease 撤销和 context/profile 清理；对 Remote Machine 配置链报告实际配置、runtime generation 和匹配的应用证据。任何报告都不能反向制造 owner 同意 |
| Billing | 通过原子资金预留、run 快照、stop observation 和精确结算决定运行资金，不以 UI 状态决定收费 |
| 页面和 safe view | 投影成员、授权、目标/已验证配置、失败与待应用原因；不得自行生成 `applied`、ACK、物理停止或隔离证据 |

这沿用现有 Agent 授权、manager/outbox 和 Node runtime 机制，不引入第二套 Agent 授权或通用编排平台。Agent 授权与 external grant 具有不同 revision/source fence，整机操作必须覆盖两者，不能用单个 Agent 的 `configure` 代表所有来源（[Agent 提交:1002](../backend/src/services/machine_access_service.rs#L1002)、[external 提交:1105](../backend/src/services/machine_external_grant_service.rs#L1105)、[runtime admission:582](../backend/src/services/remote_machine_runtime.rs#L582)）。Remote Machine 的托管配置应用仍需要独立的 machine configuration revision、runtime generation 和 Node/Controller 证据。

### 4.4 两条独立的生效链

环境模式变更必须同时收敛两条链路，不能用一条链路的成功状态覆盖另一条链路：

```text
Agent 授权链（复用非 Remote Machine 控制面）
owner 确认
  → machine_access_service::configure
  → Agent assignment revision++ 与 revocation outbox
  → AuthorityRevoke
  → Node 持久化 revision fence
  → 旧 lease、任务以及按需的 context/profile 清理
  → Node ACK

Remote Machine 托管配置链（阶段二新增/改造）
owner 确认
  → 保存目标 machine configuration revision
  → bootstrap/config 投递
  → managed runtime 应用配置
  → runtime generation 更新
  → Node/Controller 上报实际配置与版本
  → 服务端验证 applied evidence
```

建议页面至少区分以下状态：

| 状态 | 可证明的事实 |
|---|---|
| `approved` | owner 已确认服务端保存的 typed action；不代表任何一条应用链已经完成 |
| `authorization_applied` | Agent assignment 已提交，旧 Agent authority 已撤销，Node 已确认新的授权边界及要求的 context/profile 清理 |
| `machine_config_applied` | Remote Machine 的目标配置 revision、runtime generation 和 Node/Controller 实际证据已经匹配 |
| `environment_applied` | `authorization_applied` 与 `machine_config_applied` 同时成立，页面才可以把新的环境模式显示为已生效 |

现有 `delivery_pending`、`revocation_pending` 和 `quarantine_pending` 只覆盖 Agent 授权链的投递、旧执行撤销和 context/profile 清理。阶段二需要增加独立的托管配置应用状态，避免将 `AuthorityRevoke` ACK 误报为 Remote Machine 环境模式已经应用。

不支持 authority v2 的旧 Node 只有按 conversation 批量取消的较弱能力，不能提供 durable revision fence；这类 Node 不能直接进入具有 v2 语义的 `authorization_applied`，页面应显示升级/兼容性状态并继续保留相应 pending 或 legacy 限制。

## 5. 创建页调整

### 5.1 配置内容

托管创建页提供以下配置：

1. Machine name。
2. Agent separation 默认选择，沿用非托管页面的三个选项和准确语义。
3. Enable on this machine：Shell、Files、Browser、Computer。
4. 可选 Specialist 分配。
5. 对应的环境说明、数据政策和计费确认。

选择 `shared_legacy` 时必须展示共享说明并获得明确确认。当前 `acknowledge_shared_environment` 不能未经合同评估就当作新增 Agent 模式选择的完整同意记录。

本方案新增托管合同将“创建时选择 Specialist”定义为仅分配成员，所有工具默认关闭。创建成功后直接展开或导航到该 Specialist 的 Access 配置，引导完成模式和具体能力授权。实现必须显式覆盖 `automatic`、`require_separated`、`shared_legacy` 与 capability/context flags 的组合，不能继续走 feature-off 自动复制 ceiling 的分支。为新合同记录版本/来源，保留旧机器的已存授权、撤销 tombstone 和非托管兼容行为；禁止通过 create replay、resume 或 heartbeat 重新授予已撤销权限。

### 5.2 前置检查与真实探测

创建前检查平台和账号的已知条件，区分“不支持”“尚未验证”和“已经确认支持”：

- 已知不满足 required separation 条件时，提前说明原因并阻止对应配置继续提交。
- 只能在启动后完成的真实探测，应明确标为待验证。
- Machine Ready 与 Agent 授权可用分别展示；节点连接成功不代表 separated 已建立。
- 最终探测失败时保持拒绝 separated 执行，不回退为共享模式；提供 owner 可操作的修复或重新选择入口。

Machine Ready 但 Agent 探测/授权失败不是同一个状态。创建确认应说明机器运行费用按既有运行合同计算；失败不会自动取得 shared 同意，也不能凭 Agent 失败就声称机器已暂停或费用已停止。首发前需确定“修复、保留后暂停、owner 删除”的具体入口，以及自动暂停是否需要另行产品合同；在没有该合同前，只提供明确状态与已有 pause/delete 操作，不承诺自动退款或自动删除。

### 5.3 合同贯通

新增默认选择必须贯通创建请求、规范化幂等摘要、Machine 持久化、加密 bootstrap、运行时配置和节点能力上报。

- 与该选择有关的确认政策和版本需要可核验，不能只存浏览器勾选状态。
- 同一次已接受的创建重试不能更改 separation 或其他配置，也不能创建另一台机器。
- 旧请求、旧持久化意图和缺失字段不能被静默解释为新增共享授权。
- 旧机器保持已有授权；不能通过数据回填重新解释其模式。
- 对 bootstrap 的扩展必须保持既有身份、代次、完整性和密钥交付约束，并定义匹配版本的兼容策略。

需要新增**payload/config 语义版本**及最低匹配 CLI/image/template；它与加密 envelope v1、authority v2、separation evidence v4 是不同维度。当前 CLI `Secrets` 没有 `deny_unknown_fields`，可忽略新字段；当前 envelope v1 只界定加密封装，解密成功不能证明选择已经安装（[Secrets:84](../cli/src/node/managed_runtime.rs#L84)、[envelope:28](../machine/src/bootstrap.rs#L28)）。必须在投递前拒绝不兼容接收者，接收端拒绝不认识的必需配置版本，再将规范化配置摘要/版本绑定到 bootstrap 和当前代次的认证 WSS 观测；manager 验证实际选择与 ceiling 后才能标记应用。

兼容矩阵至少区分：旧机器/旧请求精确重放，新合同新建机，以及经批准的存量迁移。缺失字段维持历史含义或明确要求升级，不能推导成 shared 同意。旧 create intent/sealed delivery 不得覆盖新目标配置，旧 WSS/runtime generation 不得确认新配置。全部 API writers、bootstrap 消费者和 fleet readers 兼容且旧 writers 排空后才启用新合同；镜像被钉住的存量机器保持原行为并显示升级要求，不因后端发布或 rekey 自动升级。

## 6. 已有 Agent 的 separation 修改

### 6.1 页面流程

推荐操作路径：

```text
Remote Machine → Access → 选择 Agent → 更改环境模式
→ 阅读影响说明 → owner 确认
→ Agent 授权链与 Remote Machine 托管配置链分别收敛
→ 刷新授权状态、context/profile 清理状态和托管配置实际状态
```

Machine → Access 与 Agent → Details 使用同一套完整的授权组件、feature flag、支持状态和禁用原因。

- 切换到 `shared_legacy`：完整展示共享说明，要求主动勾选确认。
- 切换到 `separated`：展示当前支持状态和未满足条件，通过真实能力检查后才能确认。
- 仅修改 Agent capabilities 时，沿用现有直接保存能力的流程，不重复发起环境模式确认。
- owner 确认后，页面分别展示 Agent 授权 revision 的撤销/ACK 状态和 Remote Machine 托管配置的应用状态；前者完成不能直接替代后者。

### 6.2 owner 确认的后端边界

扩展第一方真人 owner 的页面确认适配，复用现有 typed action 和配置服务，保留 NyxBot action card 作为另一个入口。现有 `request_context` 已经服务端固定快照，但只返回 acknowledgement/thread 信息；把 `contextEnabled` 传对不能单独完成页面闭环。

不能直接放开普通能力保存接口的 `mode` 字段。页面确认必须具备与现有卡片相同的约束：

- 核实当前真人身份、机器 owner 管理权限和目标 Agent 的维护权限。
- prepare 时绑定目标 Agent、Node、明确目标模式、告知政策版本、当前 assignment revision，以及适用的有效机器配置 revision。
- 在服务端固定能力与 saved-login 授权快照，模式确认不能暗中扩大工具或登录权限。
- review 页展示服务端持久快照和具体增减项；不能以浏览器重传的完整 `Selection` 取代 typed action。
- commit 时重查真人身份、Agent 维护权、owner ACL、feature flag、当前有效配置及节点支持；过期或并发变更要求重新审阅。当前 `configure` 在事务前读取 flags/profile，再在事务内检查 revision；未来可变机器配置必须补相应读写围栏，不能直接把当前快照检查外推为无漂移保证。
- 模型、Agent Key、委托凭据和其他客户端不能调用此入口代替 owner 确认。
- 复用当前 Agent 执行撤销、context generation 更新和浏览器 profile 清理流程；托管环境的配置投递、runtime generation 和实际应用证据走独立的 Remote Machine 配置链。

切换界面要说明已有文件和登录态的处理方式，不承诺自动迁移共享目录或复制 Cookie。浏览器清理未完成时，继续阻止需要该清理完成的新浏览器工作。

现有真人门槛只接受无 OAuth client/API key/acting client 的第一方 Session 或 AccessToken（[login_client_context.rs:12](../backend/src/handlers/login_client_context.rs#L12)）。授权服务已有维护权/owner ACL、revision CAS、context generation、撤销 outbox 和 profile reset，可直接复用（[configure:906](../backend/src/services/machine_access_service.rs#L906)、[事务:989](../backend/src/services/machine_access_service.rs#L989)、[generation/outbox:1022](../backend/src/services/machine_access_service.rs#L1022)）。

### 6.3 首次选择、纯模式切换与应用结果

需要区分两种确认，避免把旧兼容行为描述成“所有模式确认都只改 mode”：

| 请求 | 新页面合同 |
|---|---|
| 已有明确 mode 的纯模式切换 | 保留当前 capabilities 和 saved-login 快照；部分授权和 all-off 都不得扩大 |
| 首次 `selection_required` 或旧协议授权 | 明确显示最终会写入的完整快照；新托管 membership-only 合同保持 all-off，旧协议不能实现此限制时拒绝或要求升级，不静默复制 ceiling |

当前 graphical 快照在旧协议，或首次选择且 capability editor 关闭时会使用 `option.ceiling`（[machine_access.rs:258](../backend/src/handlers/machine_access.rs#L258)）。新托管合同必须消除这条扩权路径；保留的历史兼容入口需展示真实范围，不把它伪装成纯 mode 切换，也不能为绕过关闭的 capability editor 而伪造 mode action。

页面按“待审阅 → 真人已批准 → Agent 授权提交成功/提交失败 → Agent authority 撤销与 context/profile 清理 → Remote Machine 托管配置应用 → 环境模式已生效”分别记录和显示。当前 Allow 决定先持久化，再调用 context apply，后者仍可能 revision 冲突（[assistant_nyxagent.rs:1129](../backend/src/handlers/assistant_nyxagent.rs#L1129)、[apply:1153](../backend/src/handlers/assistant_nyxagent.rs#L1153)）；因此 `approved` 不等于 `authorization_applied`，`authorization_applied` 也不等于 `environment_applied`。提交失败要显示重新审阅入口，重试复用原请求或返回明确冲突；Agent authority 尚未得到 Node 确认，或托管配置尚未得到匹配的 runtime evidence 时，都必须保留 pending。

## 7. 分开 separated 基础设施与 Browser 授权

当前 separated 的完整验证需要托管浏览器及相关执行身份；当前托管配置却把这些基础设施与 Browser 能力开关绑定。

建议分成三层：

1. **运行基础设施**：由可信镜像、平台和 separation 需求决定，准备执行身份、浏览器策略及完整探测条件。
2. **机器能力上限**：由 owner 选择是否向客户端提供 Browser、Computer 等工具能力。
3. **Agent 授权**：由 owner 决定具体 Agent 能否使用这些工具。

目标场景为：一台仅提供 Shell/File 的 Remote Machine 可以建立 separated context，而该 Agent 依然不能调用 Browser/Computer。

准备浏览器基础设施不等于授予浏览器工具。仍需通过当前完整能力证据验证，包括 Linux、执行身份、Landlock/seccomp、浏览器策略与 sandbox、凭据保管等要求，不能以“镜像包含 Chromium”或旧版 support boolean 代替。

完全不依赖浏览器的 separated 属于另外的运行时能力开发，不纳入本方案的必要前提。

### 7.1 托管账号与完整探测前置

现有 Pod 只读根、固定最小 supervisor capabilities，当前 context 动态 `useradd` 会写账号数据库；三个预建 legacy 用户也不能直接代替每 context 的 command/secure/dev 身份。需选择受限账号供给方案，例如在镜像内预建有限且不可复用的身份池，或设计可验证、supervisor 独占的受控账号数据库/重建机制。选择必须同步 image、renderer、admission 与 runtime，处理账号碰撞、容量耗尽、重建失败并拒绝执行；不能默默改为可写根、添加提权/跟踪权限或把不同 context 映射到同一用户来通过测试。这是待选工程方案，不是本轮已实施决定。

仍沿用完整 v4 证据：Linux、Landlock ABI ≥6、分离身份、真实 command seccomp/Landlock、managed browser policy 与 renderer sandbox、每身份凭据保管正向验证（[context.rs:48](../machine/src/context.rs#L48)、[证据检查:71](../machine/src/context.rs#L71)、[custody:94](../machine/src/context.rs#L94)）。`None`、v1–v3、未知版本、旧 Support boolean、包已安装或容器无法观测都不能代替完整证据。实际 RuntimeDefault/AppArmor/内核限制导致失败时保持 unavailable，不关闭 browser sandbox 或增加 capabilities 换取“可用”。需在实际 hosted 安全形态下运行至少两个同时存在的 context；native Linux 成功不自动外推到该 Pod。

### 7.2 持久 context 与临时数据

保留工作区不能只检查 `/workspace` PVC：当前 separated `command/workspace`、UID/profile-generation journal 都在临时 identity 路径，见 §3。新增合同需要一条 supervisor 保护的持久 context 布局，保留 workspace、UID 分配、generation/tombstone 与可信重建所需映射，或提供效果等价且不复用旧 UID 的可信重建。该元数据不得让 legacy Shell、任一其他 context 或外部客户端读写；节点 auth/signing/bootstrap 凭据继续留在受保护临时 identity，不能将整个 identity 目录移入 caller 可读工作区。

存储合同逐项定义：separated workspace 跨 Pod replacement/pause-resume 保留；UID/generation/tombstone 同步保留；command HOME、secure/dev browser profiles、downloads 和 tmp 仍按当前临时数据政策处理，除非另行明确批准持久化。若 profile 重建，必须推进/核验 generation，不能复制共享 Cookie 或放行旧登录填充。恢复应校验原 PVC UID、ownership、原子 journal 与目录一致性，失败保持拒绝，不能初始化成“同一工作区”的空目录。实际 CSI、重启写入和跨身份不可读仍是运行验收，不是本轮源码审阅已证明的结果。

## 8. 修正授权状态与生效反馈

`permissions_required` 不能通过“Agent 是否获得机器全部能力”判断。

示例：机器开启 Shell、File、Browser；owner 只给 Specialist 授予 File。这是有效的部分授权，应显示“已授权：File tools”，不应因缺少 Shell/Browser 持续提示权限待配置。

建议分别展示以下状态：

| 维度 | 示例 |
|---|---|
| 成员关系 | 已分配、未分配 |
| 环境模式 | Shared legacy、Separate workspace and browser、待 owner 选择 |
| 工具授权 | File tools；或未授予任何工具能力 |
| 实际可用性 | 可执行、机器离线、运行窗口到期、模式支持不可用等当前原因 |
| Agent 授权应用 | 已提交、等待 Node authority ACK、旧任务停止中、context/profile 清理中 |
| Remote Machine 托管配置 | 等待 bootstrap/config 投递、runtime 应用中、等待 Node/Controller evidence、配置已应用 |
| 环境模式最终状态 | 仅在 Agent 授权应用和 Remote Machine 托管配置都完成后显示环境模式已生效 |

Agent 能力全部关闭可以是用户的明确选择，不能因此自动补授权。页面应准确显示“未授予工具”，并提供主动配置入口。

禁止将“保存成功”“任务撤销请求已发出”“撤销等待窗口已过”统一显示为 Node authority 已确认、Remote Machine 托管配置已应用或物理运行已经停止。

`safe_view` 修复应读取有效 assignment 并分开计算模式/工具授权/实际可用性，不能改成另一种“至少一个权限才算成功”的强制授予。整机列表应继续展示成员分配进度，单个 Agent 页展示其实际授权；创建时选中的 Specialist 仅是配置入口，不能代表所有 Agent 状态。两个 feature flags 分别控制 capability editing 与新 context opt-in；已存权限始终执行，不能用一个总 `disabled` 隐藏所有原因。

## 9. 外部客户端与共享说明

Remote Machine 还服务于外部 OAuth、Agent Key 等授权来源。本方案中的 Agent separation 不自动将这些客户端归入 separated，也不扩大其现有工具范围。

- 外部客户端继续遵守当前明确授权的 Shell/File 范围。
- 新外部 grant 仍要求显式 `shared_legacy` 和完整 warning acknowledgement；`automatic`/`require_separated` 当前被拒绝。已批准 authority 的重新校验不强迫重复同意，但持续执行 live source/grant/ceiling fence（[范围:290](../backend/src/services/machine_external_grant_service.rs#L290)、[新同意:368](../backend/src/services/machine_external_grant_service.rs#L368)、[重校验:388](../backend/src/services/machine_external_grant_service.rs#L388)）。
- 某个 Specialist 使用 separated，不代表整台机器所有客户端都已 separated。
- 机器层面的共享说明应准确表达按 Agent 设置的模式，以及仍存在的外部共享环境。
- 不能仅因增加 separation 选择，就删除仍适用的共享环境告知，或将跨客户端秘密隔离声明改成 true。

## 10. 交付范围与实施顺序

### 10.1 可选范围

| 方案 | 范围 | 取舍 |
|---|---|---|
| separation 对齐、分阶段交付（推荐） | 统一页面和状态；贯通托管 separation 创建、Agent 模式确认和工具授权 | 能完成本文目标，需要扩展托管创建、bootstrap、runtime 和真实环境验证合同 |
| 仅补齐 Agent 授权页面 | 修复表单参数、禁用原因、部分授权状态和 owner 确认入口；机器上限继续创建后不可变 | 改动较小，但无法解决托管 separation 创建与运行时准备 |

阶段一的收益是立即减少授权误判和操作困惑，依赖局部组件及 read projection；阶段二的收益是完整的 owner 选择和新建机 separation，但成本跨越 API、镜像、持久存储和真实 OS 验证。这是依赖复杂度判断，不是工期或收益率估算。后续投入是否划算仍依赖真实使用频率、托管探测成功率和运行成本，当前没有数据支持数值 ROI。阶段一可先交付，阶段二按技术 gate 决定是否继续，保持本文目标可追踪。

### 10.2 阶段一：修复已有授权体验

- Access 页与 Agent Details 页传递一致的 capability/context feature 状态。
- 修正部分授权被误判为 `permissions_required` 的逻辑。
- 明确机器端未启用、账号功能未启用、模式未选择、协议需更新等原因。
- 分开展示保存、owner 批准、Agent 授权提交、Node authority 应用、撤销和 context/profile 清理状态；不在阶段一把这些状态合并为 Remote Machine 环境模式已生效。

阶段一以复用现有 Agent machine access 控制面为主：

| 可复用部分 | 复用方式 |
|---|---|
| separation 选择语义和共享环境告知 | 沿用非托管创建页的 `automatic`、`require_separated`、`shared_legacy` 语义和 warning；参考 [machine-setup.tsx:276](../frontend/src/pages/machine-setup.tsx#L276) 和 [machine-setup.tsx:294](../frontend/src/pages/machine-setup.tsx#L294) |
| Agent machine access API | 继续使用 `/assistant/nyxagent/agents/{id}/machines` 的通用 GET/PUT/context-request 路由，以及 `machine_access` handler；Remote Machine 节点已经通过同一 Node machine profile 进入这条路径（[routes.rs:2635](../backend/src/routes.rs#L2635)） |
| 前端能力编辑和状态刷新 | 复用 `MachineCapabilityForm`、`useMachineAccess`、`useSetMachineAccess`、`useRequestMachineContext`、`useAppForm` 和现有 feature flags；Remote Machine Access 页面已经嵌入该表单（[remote-machine-agent-access.tsx:300](../frontend/src/components/machines/remote-machine-agent-access.tsx#L300)） |
| 授权一致性与撤销状态 | 复用 `machine_access_service::configure` 的 owner ACL、revision CAS、capability ceiling、saved-login 校验、context generation、撤销 outbox，以及 `approved`/`authorization_applied` 的状态区分（[machine_access_service.rs:899](../backend/src/services/machine_access_service.rs#L899)）；阶段一不把这条链的 ACK 解释为 hosted environment 已应用 |

阶段一仍需要 Remote Machine 专属适配：

- 修正 [remote_machine_configuration.rs:128](../backend/src/services/remote_machine_configuration.rs#L128) 附近 `safe_view` 对全部机器 capability 使用 `all()` 的判断，正确显示部分授权和全关闭状态。
- 调整 [remote_machine_configuration.rs:78](../backend/src/services/remote_machine_configuration.rs#L78) 的 Specialist 初始分配，使新托管合同只建立成员关系、工具权限默认关闭，并保留旧机器的既有授权。
- 继续使用 Remote Machine 自己的 Access 页面、托管状态和 assignment 错误投影；不能把本机 CLI 的 setup command 当作阶段一的托管创建流程。
- 如果页面展示环境模式状态，阶段一只展示已有托管状态和 Agent 授权链状态；没有目标 machine configuration revision、runtime generation 和实际应用证据时，不能显示 `machine_config_applied` 或 `environment_applied`。

### 10.3 阶段二：打通 separation

- 先对匹配新 image/template 的新建机器贯通默认选择、payload/config 版本和 membership-only 授权；既有 Agent 通过 owner 流程切换，前提是其机器已具备完整支持。
- 第一方 owner 在 Access 页完成模式确认，保留同等授权边界。
- 拆开 separated 基础设施与工具能力授权。
- 完成受限账号供给、持久 context/journal 和真实 hosted v4 正反向探测，尤其验证 Shell/File-only 的完整 infra 方案。
- 保持已有授权不被默认选择、升级或 feature flag 重新解释。

阶段二复用非 Remote Machine 的授权控制面和 owner 流程，但不直接复用本机安装和运行时实现：

| 可复用部分 | 复用方式 |
|---|---|
| separation choice 和模式解析 | 沿用 `automatic`、`require_separated`、`shared_legacy` 的既有选择合同；使用同一套 `separated_support`/完整 evidence 语义，缺少证据时保持拒绝或要求 owner 选择 |
| owner 确认流程 | 复用 `request_context` 创建 typed action card、服务端固定 capability/saved-login 快照，以及 [apply_human_context_action](../backend/src/handlers/machine_access.rs#L316) 调用的 fenced `configure` 事务；页面确认不能另造一套授权权威 |
| 授权 revision 和执行围栏 | 复用 Agent assignment、external grant 的 revision/source fence、撤销 outbox、context generation 和 browser profile cleanup；这些状态只覆盖 Agent 授权链，不能单独代表 Remote Machine 环境已经应用 |
| Remote Machine 配置应用 | 新增/改造 machine configuration revision、bootstrap/config 投递、managed runtime generation、Node/Controller 实际配置上报和 applied evidence；它与 Agent assignment revision 分开存储、校验和展示 |
| 通用页面和查询 | 复用 `MachineCapabilityForm`、machine access schema、owner mutation guard、Agent 授权状态刷新和 `approved`/`authorization_applied` 展示；托管页面还要查询并投影 Remote Machine 的 `machine_config_applied`，两者都完成后才显示 `environment_applied` |

阶段二必须新增或改造 Remote Machine 专属合同：

- 在 [remote_machine.rs:85](../backend/src/handlers/remote_machine.rs#L85) 的 hosted `CreateRequest`、前端 hosted schema、规范化幂等摘要和 Machine 持久化中加入 separation choice；当前托管创建请求只有 capability、Specialist 和 shared-environment acknowledgement，不能直接接受非托管 setup 页的字段。
- 将 choice 贯通到 bootstrap payload/config 语义版本、managed CLI 配置、runtime generation、Node 上报和版本兼容矩阵；解密成功或旧 WSS 连接不能单独证明新选择已经应用。
- 保留 Remote Machine 自己的 `remote_machine_configuration`、bootstrap、managed runtime、hosted image、Pod/admission 和 billing/lifecycle 流程；非托管的 [machine-setup.tsx:841](../frontend/src/pages/machine-setup.tsx#L841) 生成本机 CLI 命令，不能直接替代托管 bootstrap。
- 完成托管 Pod 的受限账号供给、持久 context/journal、PVC 恢复和真实 hosted v4 正反向探测；native Linux 或本机 CLI 的 separation 成功不能外推为托管 separated 已可用。
- 将 Agent 授权链与托管配置链的状态分开持久化和重试：`AuthorityRevoke` ACK 只能结束 Agent delivery/revocation pending；只有目标 configuration revision、runtime generation 和 Node/Controller evidence 同时匹配，才结束 machine configuration pending。

阶段二边界：已有 Machine 的默认选择也在严格校验的 runtime config 内，不能通过数据库设置立即生效；存量默认设置或旧 image/template 迁移需要另行批准的配置/迁移合同。首批可先交付新建机选择和已有 Agent 切换；旧镜像未升级的机器显示不支持/待升级，原授权继续按原合同执行。Agent 授权链完成而托管配置链未完成时，页面保持 `authorization_applied`/`machine_config_pending`，不能合并显示为环境模式已生效。

阶段二上线 gate 为 §5.3 的版本矩阵、§7 的账号/持久化/完整证据，以及首次选择不扩权。未通过 gate 时，仍可独立实现并交付阶段一，不降低 separated 的既有安全含义。

实施时沿用 handlers → services → models 分层、现有 TanStack Query hooks、Zod schemas、`useAppForm` 和 DESIGN.md 规定的组件及状态表达。

## 11. 验收标准

以下为计划验收项，不是已完成的测试结果。

### 11.1 页面、服务端与兼容性

| 场景 | 预期结果 |
|---|---|
| 仅启用 Shell/File，选择 Automatic，分配 Specialist 且只授权 File | 条件满足时使用 separated；授权状态完整；Shell、Browser 和 Computer 调用被拒绝 |
| Automatic 缺少有效能力证据 | 显示明确原因并等待选择，不静默变成 shared legacy |
| Require separated 最终探测失败 | 不以共享模式执行，也不显示 Agent 授权可用 |
| owner 显式选择 shared legacy | 确认绑定正确 Agent、机器、模式和当前授权；可以在 Access 页完成 |
| 模式确认期间授权 revision 已变化 | 拒绝旧确认，要求重新审阅 |
| 只切换环境模式 | 不暗中扩大 capabilities 或 saved-login 授权 |
| 机器开启多项能力，Agent 只获得 File | 显示有效的部分授权，不要求全部授予 |
| owner 将 Agent 能力全部关闭 | 显示未授予工具，不自动补授权 |
| 节点离线、撤销或浏览器清理未确认 | 保留 Agent 授权链待应用状态，不伪造 ACK 或停止证据；托管配置链也继续显示独立 pending |
| Node 已确认 `AuthorityRevoke`，但 hosted runtime 没有匹配配置证据 | 显示 `authorization_applied` 与 `machine_config_pending`，不显示 `environment_applied` |
| Agent 授权和托管配置都已确认 | 只有两条链的目标 revision、runtime generation 和 evidence 匹配后，才显示 `environment_applied` |
| 修改新 Agent 的默认选择 | 已有 Agent 模式、权限和登录限制保持原样 |
| 账号功能关闭或节点协议不支持 | 显示具体原因；新增操作不绕过检查，已有约束继续生效 |
| 外部 OAuth / Agent Key 客户端 | 不获得 Browser/Computer 或未经批准的 separated 权限 |
| 旧创建意图与混合版本 | 不把旧勾选或缺失字段升级为新同意；新合同在匹配版本完成部署后启用 |

以下负向验收是新增合同的必要补充；单纯 mock “保存成功”不能证明满足：

| 优先级 / 场景 | 必须验证的结果及证据层 |
|---|---|
| P1：新托管三种 choice × 两个 flags × 首次/已有模式 | 新建 membership-only 的工具均关闭；首次 unresolved/旧协议不得继承 ceiling；纯 mode 切换保持 partial/all-off/login 快照；flag-off 不扩大已有权限。服务层和真实 mounted API 同时覆盖 |
| P1：prepare 后变更授权、机器有效配置、policy、owner/维护权、flag 或节点 evidence | commit 拒绝过期确认，提示重审；不能将 `approved`、`authorization_applied` 或 revision 冲突错误显示为 `environment_applied`，也不接受伪造 `Selection`/真人身份 |
| P1：旧 bootstrap 消费者、缺字段、未知配置版本、旧 image/template | 在投递/安装/准入处拒绝或要求升级，不忽略 separation 后形成共享同意；保持正确旧意图的精确重放，不能重放旧配置覆盖新目标 |
| P1：当前 generation 改变后旧 WSS/delivery/lease 或旧配置 evidence 到达 | 旧观测不能确认新配置，旧 lease 不能续成新 runtime 权限；配置仍待对应代次的认证验证 |
| P1：离线节点、撤销到期、ACK 与 quarantine 分离 | lease 界内拒绝续租并收敛执行等待；delivery 无 ACK 仍 pending，quarantine 无清理确认不解除；不能报告物理停止或撤回已完成外部效果 |
| P2：Machine Ready、Agent 探测失败 | UI 区分两者，显示收费与暂停/修复/删除入口；不伪造 shared 同意、免费暂停、自动退款或数据删除 |

### 11.2 真实 runtime 与选定集群

| 优先级 / 场景 | 必须取得的运行证据 |
|---|---|
| P1：真实 hosted 形态，Shell/File-only + 完整 infra | 使用正式只读根、最小 capabilities、RuntimeDefault/AppArmor 和实际镜像；两个同时存在的 separated contexts 各自完成 workspace 与 secure/dev browser 探测；Agent Browser/Computer API 仍被拒绝 |
| P1：账号供给/碰撞/耗尽/重建失败 | 按受限供给方案成功或显式拒绝；不共用 UID、不复用 tombstone、不增加 privileges、不以关闭 sandbox 通过探测 |
| P1：跨 context 与 credential custody | 用每个实际 worker 身份验证兄弟 workspace、node credentials、journal、browser/profile/socket 不可读写及旧 generation 不可用；包括 legacy/外部共享身份，不能只检查 mode bits |
| P1：Pod replacement 与 pause/resume | 在 separated workspace 写 marker，替换后从原 PVC 与相同逻辑 context 读取；UID/generation/tombstone 可验证地连续，临时 HOME/browser/tmp 按说明丢弃/重建，旧登录填充和旧 lease 被拒绝 |
| P1：负向探测与混版 | 隐藏 sandbox 观测、旧 v1–v3/未知证据、custody/ABI/policy 失败均 unavailable，不回退共享；arm64/amd64 与实际内核分别验证 |
| P1：集群与费用链路 | 验证真实 controller、RuntimeClass/RuntimeDefault、CSI/PVC 原卷保留及恢复、实际 Node/client 鉴权、stop observation 与 Billing；本地 HTTP/签名替身不替代这些结果 |

涉及执行和权限的验收应同时覆盖服务端校验与节点实际行为。静态类型检查、模拟响应或前端状态更新不能代替真实 separated、权限撤销或持久工作区验证。

## 12. 实施涉及的主要代码

| 范围 | 主要位置 |
|---|---|
| 托管创建表单与请求 schema | [remote-machine-create.tsx](../frontend/src/components/machines/remote-machine-create.tsx)、[machines.ts](../frontend/src/schemas/machines.ts) |
| 机器详情与 Access 页 | [remote-machine-detail.tsx](../frontend/src/components/machines/remote-machine-detail.tsx)、[remote-machine-agent-access.tsx](../frontend/src/components/machines/remote-machine-agent-access.tsx) |
| 通用 Agent 能力组件及 hooks | [machine-capabilities.tsx](../frontend/src/components/assistant/machine-capabilities.tsx)、[use-machine-access.ts](../frontend/src/hooks/use-machine-access.ts) |
| 托管 HTTP 合同、持久化和配置状态 | [remote_machine.rs](../backend/src/handlers/remote_machine.rs)、[remote_machine.rs（model）](../backend/src/models/remote_machine.rs)、[remote_machine_configuration.rs](../backend/src/services/remote_machine_configuration.rs) |
| owner 模式确认与授权服务 | [machine_access.rs](../backend/src/handlers/machine_access.rs)、[machine_access_service.rs](../backend/src/services/machine_access_service.rs) |
| 托管生命周期与 bootstrap | [remote_machine_service.rs](../backend/src/services/remote_machine_service.rs)、[remote_machine_manager.rs](../backend/src/services/remote_machine_manager.rs)、[remote_machine_bootstrap.rs](../backend/src/services/remote_machine_bootstrap.rs) |
| 节点托管配置与载荷安装 | [managed_config.rs](../cli/src/node/managed_config.rs)、[managed_runtime.rs](../cli/src/node/managed_runtime.rs)、[config.rs](../cli/src/node/config.rs) |
| Context 机制、账号与存储 | [context.rs](../machine/src/context.rs)、[context_runtime.rs](../cli/src/node/machine/context_runtime.rs)、[context_dispatch.rs](../cli/src/node/machine/context_dispatch.rs) |
| 镜像、Pod 与部署准入 | [Dockerfile.machine-hosted](../cli/Dockerfile.machine-hosted)、[hosted-entrypoint.sh](../cli/container/hosted-entrypoint.sh)、[sandbox.rs](../machine-updater/src/sandbox.rs)、[admission.yaml](../deploy/remote-machine/admission.yaml) |
| 全来源权限与运行资金 | [machine_external_grant_service.rs](../backend/src/services/machine_external_grant_service.rs)、[machine_external_service.rs](../backend/src/services/machine_external_service.rs)、[remote_machine_runtime.rs](../backend/src/services/remote_machine_runtime.rs)、[remote_machine_operation.rs](../backend/src/models/remote_machine_operation.rs) |

## 13. 相关合同与验证边界

- [Machine 节点合同](MACHINE_NODES.md)。
- [Agent separation 合同](MACHINE_AGENT_ISOLATION.md)。
- [节点安装、默认选择与能力配置](NYXID_NODE.md)。
- [Remote Machine 当前合同](REMOTE_NYX_MACHINE_FINAL_READ_CONTRACTS.md)。
- [Remote Machine MVP 与后续调整记录](REMOTE_NYX_MACHINE_MVP_PLAN.md)。
- [既有 hosted image/storage 本地验证及其环境限制](REMOTE_NYX_MACHINE_HOSTED_STORAGE_VERIFICATION_20261015.md)。
- [项目设计系统](../DESIGN.md)。

既有 hosted image smoke 的 authority 明确为 `shared_legacy`（[smoke:285](../deploy/remote-machine/tests/full-machine-image-smoke.py#L285)）；其中 fresh context/revision 检查不是 OS separated 验证。此前 storage 记录使用真实本地 Node/Chromium/cua，但 bootstrap/WSS、Kubernetes 与支付有替身，实际镜像验证限 arm64，CSI/RuntimeDefault/真实客户端仍待选定环境验收（[验证边界:124](REMOTE_NYX_MACHINE_HOSTED_STORAGE_VERIFICATION_20261015.md#L124)）。既有 native separated 矩阵可供复用，不代表 hosted Pod 已通过相同测试。

本轮完成范围仅为源码核对和本文修订及文档链接/锚点/一致性检查，未构建、运行产品测试、Node、镜像或真实集群。本文记录调整建议及计划验收要求；文档写入不改变既有产品合同、运行时配置、功能开关或用户授权，也不代表功能实施、集群验证或发布完成。账号供给方案、已有机器默认选择/镜像迁移和探测失败处置是实施前需要明确的选择。
