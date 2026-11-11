# Remote Machine：由 Sandbox 生命周期驱动删除与权限收敛方案

日期：2026-10-20

状态：待实施的设计方案。本文记录本次讨论结论，不表示代码、RBAC 或线上环境已完成调整。实施前现行行为仍以 `docs/REMOTE_MACHINE_DELETE_RECOVERY.md` 为准；本方案不能作为提前撤销权限的依据。

## 1. 目标与决策

Manager 仅控制原始 Sandbox 的暂停和删除，不再主动删除 Pod/PVC，不再通过集群级 PV、VolumeAttachment 枚举证明存储清理完成。Agent Sandbox controller 管理工作负载，Kubernetes GC 根据 ownerReference 回收子对象，存储控制器与 CSI 负责后续存储处理。

保留当前 typed List 兼容修复，以及 NyxID 创建责任收敛、原始对象身份、停止证据、计费幂等、并发隔离和事务终态检查。

完成实现与验收后，Manager 不需要以下权限：

| 权限 | 目标状态 |
|---|---|
| Pod delete，限 `nyxid-machine` | 撤销 |
| PVC delete，限 `nyxid-machine` | 撤销 |
| PV list，集群级 | 撤销 |
| VolumeAttachment list，集群级 | 撤销 |

这里的权限收敛只针对 Manager。不能将 Manager 的撤权同时应用到 Agent Sandbox controller、GC 或存储组件。

## 2. 已核实的上游行为

### 2.1 版本与源码依据

本次源码核对覆盖 NyxID 部署文档固定的 commit `86ad94178a85aa26788077a55894cdc234c5ea6a`，以及查询时上游 HEAD `2f4747721a779b9427e2282c3bb1ecf739add653`。本节核心行为在两者一致。实际集群 controller 镜像、CRD、部署权限和存储行为仍需验收，不能由源码检查推定。

- [固定版本 controller 源码](https://github.com/kubernetes-sigs/agent-sandbox/blob/86ad94178a85aa26788077a55894cdc234c5ea6a/controllers/sandbox_controller.go)
- [Suspended condition](https://github.com/kubernetes-sigs/agent-sandbox/blob/86ad94178a85aa26788077a55894cdc234c5ea6a/controllers/sandbox_controller.go#L578)
- [暂停时删除所属 Pod](https://github.com/kubernetes-sigs/agent-sandbox/blob/86ad94178a85aa26788077a55894cdc234c5ea6a/controllers/sandbox_controller.go#L1328)
- [模板 PVC 创建与 ownerReference](https://github.com/kubernetes-sigs/agent-sandbox/blob/86ad94178a85aa26788077a55894cdc234c5ea6a/controllers/sandbox_controller.go#L1771)
- [Sandbox 进入删除状态后退出 reconcile](https://github.com/kubernetes-sigs/agent-sandbox/blob/86ad94178a85aa26788077a55894cdc234c5ea6a/controllers/sandbox_controller.go#L308)

### 2.2 PVC ownerReference 已由 controller 设置

NyxID 当前通过 `spec.volumeClaimTemplates` 声明 workspace。Agent Sandbox controller 从模板创建 PVC，在创建前调用 `ctrl.SetControllerReference(sandbox, pvc, r.Scheme)`，绑定实际 Sandbox UID。Pod 也设置所属关系。

因此，当前模板创建路径无需 NyxID 新增 PVC ownerReference 写入代码。已有 PVC 是否正确归属必须检查实际对象；人工创建、历史遗留、被外部修改或不属于原 Sandbox 的对象，不能假设会被级联回收，更不能直接补归属后删除。

### 2.3 暂停与删除不是同一条清理路径

请求 `operatingMode: Suspended` 后，controller 检查 Pod 所属关系并删除属于当前 Sandbox 的 Pod。Pod 不存在且查询无错误时，才报告 `Suspended=True`、reason `PodTerminated`，condition 携带 `observedGeneration`。PVC 保留用于恢复。

Sandbox 出现 `deletionTimestamp` 后，controller 直接退出 reconcile，依赖 GC 处理所属资源。上游没有等待 PV/VolumeAttachment 清理完成的删除 finalizer，也没有对应的存储释放证明。声明了 `sandboxes/finalizers` RBAC 不代表实现了该证明。

`Suspended=True` 是控制面 Pod 观察，不是物理进程停止时刻、卷 detach 或磁盘擦除证明。不得据此放宽现有其他运行安全门槛。

## 3. 组件职责与权限身份

| 组件 | 责任 | 使用的身份 |
|---|---|---|
| Manager | 用户授权、创建责任、原 Sandbox 暂停/删除、操作状态、计费、重试与身份校验 | Manager 的 KubernetesConnection 实际凭证；采用 SA 时为 Manager SA |
| Agent Sandbox controller | 根据模板创建 Pod/PVC，设置 ownerReference，暂停时删除所属 Pod并更新 condition | controller 自己的 ServiceAccount |
| Kubernetes GC | 按 ownerReference 和传播策略回收所属 Pod/PVC | 集群控制面的 GC 身份 |
| 存储控制器与 CSI | 挂载关系协调、detach、按 reclaim policy 回收 PV 与底层卷 | 各组件的 Kubernetes 身份及所需云端身份 |

后续组件不继承 Manager 身份。Manager 删除 Sandbox 不要求它拥有 Pod/PVC delete；Manager 请求暂停不要求它拥有 controller 的 Pod delete；存储回收也不要求 Manager 获得 CSI 或云盘权限。

Manager 与 Agent Sandbox controller 必须使用独立身份及绑定。检查所有 RoleBinding/ClusterRoleBinding 的有效权限并集；共用 SA、其他宽权限绑定或使用管理员凭证，会使本次收敛失效。

## 4. 目标权限边界

| 资源与范围 | Manager 目标权限 | 说明 |
|---|---|---|
| Sandbox，原命名空间 | get/list，按现有调用保留 watch/create/patch/delete 等 | 本表不是完整角色替换清单；逐项核实 update/status 等现有用途 |
| Pod，原命名空间 | 保留所需 get/list/watch | 支持现有身份、安全、只读残留诊断；不授予 delete/create/patch/exec |
| PVC，原命名空间 | 保留所需 get/list/watch | 保留恢复时 workspace UID 等检查；不授予 delete/create/patch |
| Pod，集群级 | 不需要 list | 删除流程不应请求 `/api/v1/pods` |
| PV，集群级 | 不需要 list | get 若被其他功能使用，应单独审计，不因本方案自动撤销或新增 |
| VolumeAttachment，集群级 | 不需要 list | 删除路径不再需要 get；其他用途与 healthcheck 一起审计 |
| Node、Lease、StorageClass、TokenReview 等 | 本方案不自动变更 | 保留现有暂停/恢复、身份或准入功能实际需要的权限 |

Foreground 删除、ownerReference 的 `blockOwnerDeletion` 及相关 admission 授权需要在目标集群验证。不能以增加 Manager 的 Pod/PVC delete 或存储 list 权限掩盖传播策略或 admission 配置问题。

## 5. 删除流程与完成语义

以下为逻辑阶段，不要求直接新增同名数据库枚举。实施时映射现有状态，并显式版本化变化后的证据契约。

### 5.1 接受删除并禁止后续执行

校验 actor/owner 和原删除 operation，禁用执行与新创建入口。所有写入继续绑定 owner、operation、runtime generation、credential epoch、revision、租约与完整 writer fence。重试不创建新的删除操作或财务身份。

### 5.2 收敛 NyxID 创建责任

保留 outbox、每次外部派发记录和晚到结果处理。只有确实未派发的请求才能按现有事务条件取消。超时或响应丢失不得由 Sandbox 404、Pod 消失、重试成功或存储为空推断为从未创建。

对晚到创建结果先记录原始责任与对象身份，再按原操作处理。可识别的晚到原 Sandbox 通过 Sandbox API 清理；不得采用同名不同 UID 的替代对象。未终结的创建责任继续阻止最终成功和槽位释放。

### 5.3 停止原 Sandbox

原 Sandbox 存在时，采用 UID/resourceVersion 保护的最小 Suspended patch，保留工作区与 Pod 模板。等待实际原对象的 `Suspended=True / PodTerminated`，验证当前 condition observedGeneration 和操作绑定，保留现有必要的安全观察。

原 Sandbox 已不存在、部分创建或没有运行记录时，保留有依据的缺席分支。创建责任必须已终结，保留所需命名空间 Pod 只读检查及已知原始身份判断；单次 404 不能直接结束删除。无法建立适用停止/缺席证据时继续 pending。

### 5.4 发起明确的级联删除

对原 Sandbox 发起带 UID/resourceVersion preconditions 的 DELETE，显式设置 `propagationPolicy: Foreground`。Manager 不删除 Pod/PVC，不调用存储 list，也不使用 Orphan、force delete、finalizer 清除或 reclaim policy 改写绕过阻塞。

DELETE 接受不是完成。持续观察原 Sandbox 删除进展；409 后重新观察与校验，不能无条件重试删除同名对象。404 只表达此次原对象读取/删除的缺席，仍需组合创建责任、身份、适用工作负载和计费检查。

### 5.5 验证对象级删除并完成事务

Foreground 由 GC 等待其能够跟踪、且 `blockOwnerDeletion` 生效的依赖对象。它不是“所有后端资源已释放”的原子证明，不涵盖未正确归属、未被观察到的对象或底层磁盘。

保留命名空间 Pod/PVC 只读诊断。已知原始子对象仍存在时记录并保持对象清理 pending，由 GC/运维处理，Manager 不直接删除。遇到替代 UID、foreign owner 或无法可靠关联的资源时不接管、不误删，保留相应诊断及安全阻塞。

最终成功要求原删除操作与身份仍有效、执行禁用、NyxID 创建责任终结、适用停止/缺席及对象删除检查通过、计费结算完成。最终状态、operation 成功、凭证撤销和槽位释放继续按原事务与幂等约束提交。

### 5.6 存储完成语义明确收窄

新方案不再要求 Manager 证明相关 PV/VolumeAttachment 无残留，也不再把该证明作为 deleted 的必要条件。不得将“未查询”保存为“查询结果为空”，不得伪造现有 `ControlPlaneReleased` 存储证据。

建议用户文案：机器已删除，工作负载及工作区通过 Kubernetes 级联回收，底层存储按回收策略处理。不能承诺磁盘已经擦除或所有存储对象已消失。

若产品必须继续承诺 PV/attachment 已清理，需要独立的受信任观察组件及可验证结果；这属于额外方案，当前上游不能直接提供。

停止计费沿用首次适用且正确归属的控制面停止/缺席观察边界，不因 GC/存储等待或重试不断后移，也不由本方案引入物理停止证明。账务失败继续阻止财务终态，但不阻止独立安全的 Sandbox 清理。

## 6. typed List 修复与已知阻塞

保留 `machine-updater/src/sandbox.rs::normalize_recovery_list_item`。只有外层 typed List 已验证时，才允许 item 缺失的 kind/apiVersion 继承类型；显式 null、错误类型和冲突继续拒绝。命名空间、UID、ownerReference、分页完整性、体积/时间边界等校验继续保留。

| 原阻塞 | 新方案效果 |
|---|---|
| Manager 缺 Pod/PVC delete | 移除调用后消除该权限依赖 |
| Manager 缺 PV/VolumeAttachment list | 移除查询和完成门槛后消除该权限依赖 |
| 合法 typed List item 被旧解析器拒绝 | 当前兼容修复直接处理；架构调整仅消除被取消查询的触发点 |
| PVC protection、子对象 finalizer、GC 故障 | 仍可能 pending，交给 GC/运维，不强行删除 |
| Retain PV 或孤立存储 | 按策略保留/告警，不由 Manager 直接回收；不能声称磁盘已删 |
| 创建责任未知、晚到创建风险 | 继续阻塞相关终态，依赖原创建责任协议 |
| 财务失败、UID 冲突、并发冲突 | 保留对应状态机和校验 |

所有剩余 List 调用仍需兼容解析。仅删除 DELETE 调用而保留旧前置存储枚举，不能解决原先的查询阶段阻塞。当前 checkout 有修复不代表运行中的二进制已包含修复。

## 7. 残留观察与异常处理

终态后的持续扫描也必须改造，不能继续进入旧 `recover_resources` 并触发子对象 DELETE 或集群存储 LIST。保留原始 Sandbox 责任跟踪、已知命名空间对象的只读诊断和公平有界调度。

晚到原 Sandbox 按身份清理，晚到所属 Pod/PVC 交给 GC。已完成状态下出现残留只更新清理诊断，不重新计费、不重复释放槽位、不恢复执行或重开已成功操作。新证据契约如何表示对象清理 pending 与最终 machine 状态，需要与现有 tombstone 行为一致。

GC 停止、ownerReference 缺失、对象 finalizer 长时间不释放、CSI detach/回收异常，由平台运维监控接管。存储告警不能依赖已撤权的 Manager 枚举。沿用现有重试预算、超期升级和脱敏诊断，明确需要用户重试还是运维处理。

## 8. 实施范围与顺序

| 位置 | 修改要求 |
|---|---|
| `machine-updater/src/sandbox.rs` | 保留 typed List 修复；为 Sandbox 的身份保护删除显式设置 Foreground；移除 Manager 对旧子对象删除/存储枚举入口的使用，避免影响其他独立调用者 |
| `backend/src/services/remote_machine_delete_recovery.rs` | 重构删除前置发现、恢复与最终判定，移除旧存储清理依赖；保留创建、停止、身份和财务门槛 |
| `backend/src/services/remote_machine_manager.rs` 及相关路径 | 审计普通删除、部分创建、重试、晚到结果与终态扫描，确保没有绕回旧路径 |
| 删除证据模型及消费者 | 新契约显式版本化；历史 PV/CSI 字段只保留历史意义，旧证据不能隐式证明新契约，反之亦然 |
| `deploy/remote-machine/manager-role.yaml` | 移除 Pod/PVC delete，保留实际需要的命名空间读取 |
| `deploy/remote-machine/identity-review-role.yaml` | 移除 PV list、VolumeAttachment list 及多余集群 Pod list；get 等权限按调用审计处理 |
| `deploy/remote-machine/healthcheck.sh` 与 contracts | 校验新契约；验证四项权限拒绝、必要读取和 Sandbox 操作允许，禁止用管理员身份代替实际 Manager 身份 |
| 删除恢复、Manager、事件、重试及 updater 测试 | 更新完成语义，加入负面 API 调用断言，保留并发/账务安全覆盖 |
| 部署 README、现行删除恢复文档、API/UI 文案 | 同步新职责、权限矩阵、完成语义及运维边界 |

推荐先完成代码、契约和测试，再在独立身份的本地/预生产集群验证，最后撤销部署权限。滚动升级期间旧 worker 仍可能要求原权限并消费旧证据，必须明确兼容策略或排空旧 worker；不能先撤权，也不能让旧消费者误解释新的完成标记。不得直接改数据库将原 pending 标成成功。

## 9. 验收要求

### 9.1 确定性测试

- 验证 Sandbox DELETE 同时包含 UID/resourceVersion preconditions 和 Foreground，404/409/超时不伪造成功。
- 对正常删除、部分创建、重试和终态扫描断言：没有 Pod/PVC DELETE、PV LIST、VolumeAttachment LIST 请求。
- 保留合法 typed List item 省略 TypeMeta 的成功用例；错误/null/冲突、跨命名空间、损坏身份和不完整分页必须失败。
- 覆盖原 Sandbox 缺席、同名替代 UID、foreign owner、过期 condition、晚到原 Sandbox、创建责任未终结等路径。
- 保留计费故障、并发 claim/revision、重复删除、重试预算、终态事务及不重复结算/释放槽位的测试。
- 验证 pending 旧记录和历史证据的迁移/兼容策略，证明不会重新启用执行或凭空产生存储释放证明。

### 9.2 真实 Kubernetes 验收

- 记录 API server、Agent Sandbox 镜像/commit、CRD、GC、StorageClass 和 CSI 配置；本地结果不视为 GKE 等价验收。
- 以实际 Manager 身份验证四项权限均拒绝，Sandbox 必要权限及命名空间只读权限允许；确认 controller 使用独立身份。
- 验证模板 PVC 的 ownerReference 指向正确 Sandbox UID，并记录 blockOwnerDeletion；验证 Foreground admission 和 GC 的实际行为。
- 创建、bootstrap、暂停、恢复回归：暂停 Pod 消失，PVC UID 保留；恢复使用原 PVC。
- 删除时观察 Sandbox、Pod/PVC 的级联状态与 Manager 终态，确认没有 Manager 子对象直接删除调用。
- 模拟 GC/controller 不可用、PVC protection/finalizer 阻塞、部分创建和晚到结果，验证超时与诊断，确认不会靠强删绕过。
- 确认 PV/attachment/云盘回收交由对应监控覆盖，Manager 不把未观察状态宣称为存储已释放。

## 10. 本次交付范围

本次仅新增方案文档。没有修改运行代码、数据库、权限清单或线上资源，没有宣称实现测试或集群验收已通过。
