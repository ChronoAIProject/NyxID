# CMA Worker v1：外部依赖与实施记录

本文件跟踪实现方案的边界和实际验收条件，不把尚未完成的工作计作已完成。

用户于 2026-11-04 明确确认：按文档推进 **M1 + M2 + M3，能在本仓库完成的都做**。M3 属于任务范围；其中需要 CMA 代码、账号策略或目标部署的工作，保留为待配合的验收条件。

## M3 在 NyxID 仓库中的接入

| 方案项 | 本仓库已有实现与检查 | 目标 CMA 仍需提供的证据 |
| --- | --- | --- |
| §7.1 单账号 Sandbox 配额 | 最终派发层的创建准入、跨 chat/key/连接别名占位、完整远端资源发现、历史超额展示及逐个退休；事务测试覆盖并发占位和未知结果保留。 | effective owner limit = 1；直接 CMA 调用、lifecycle、manual、launch 等入口与 NyxID 并发时共用该上限。 |
| §7.2 事件与 Desktop | transcript、v2 Agent snapshot/SSE、Responses 状态/事件消费端，独立 RFB 中继，first-party/Origin/实时权限检查，以及公开 overlay 中的 `stop_agent`。本地协议和页面测试已记录。 | 目标版本的真实身份透传、游标/重连、共享历史边界、浏览器会话、RFB 连接和 viewer 冲突验收。 |
| §7.3 可选 callback_url | 已有 authenticated observation 消费、URL 校验、任务卡外链和内部 Agent 查看路径；没有 callback 也能在 NyxID 查看执行。 | CMA 在 lifecycle、Agent detail、Responses GET/SSE 中按合同生产字段，使用其配置的 public origin。 |
| §10 工具发现 | 托管 `/api/v1/catalog-specs/cma/openapi.json`，registry 测试检查 25 个 operationId 及动作分级。 | 管理员将目标 `cma` catalog 的 `openapi_spec_url` 指向 overlay，验证实际发现结果；保留已配置 URL，不自动覆盖。 |

## M3 待 CMA 或目标部署配合

- CMA effective owner Sandbox quota = 1：方案 §4.1、§7.1。需要目标 CMA 部署验证 lifecycle、manual、launch 等创建入口共用 owner 配额；NyxID 本地原子占位不能约束用户直接在 CMA 创建资源。本任务不修改 CMA 全局配额，避免影响其他账号。
- CMA `callback_url` 生产端：方案 §7.3。需要 CMA 发布；NyxID 内部执行页不能依赖此字段。现有 watch 仍负责向原 chat 回传结果。
- 真实 CMA 事件和 RFB 验收：本地 API/WS fixture 与解析测试验证 NyxID 行为，目标部署的实际连接、身份对应与 viewer 竞争仍需配对验证。既有 `serve_joined_cma_acceptance_fixture` 是资源请求证明的专用配对夹具，也不能替代本方案的完整桌面和配额验收。

## 方案明确排除的扩展

- Remote Machine 删除、Agent 池/自动回收、Sandbox 级 Desktop/exec/files、独立云端算力计费：方案 §1 明确排除，不在本次实现。

## 已发现的合同差异

- **首次登录后的幂等重试**：方案 §4.2、§6 要求保留原 body 与同一个 Idempotency-Key。本机 CMA 源码 `api/crates/layer1-public/src/rest/problems.rs` 的 `SANDBOX_CREDENTIAL_SETUP_DETAIL` 则要求对已被拒绝的 keyed command 使用新的 Idempotency-Key。NyxID 不自动换 key 或悄悄新建资源；同 key 恢复的端到端验收需要 CMA 合同对齐。在目标版本确认前不能宣称这一场景已验收。
- **桌面协议**：CMA Agent Desktop 是 RFB 二进制 WS，NyxID Machine 桌面是另一种协议。CMA 使用固定版本 noVNC 与独立授权中继，并沿用 first-party 身份、精确 Origin、期限和资源复查规则。

## 重复阻塞规则

按用户要求，同一问题反复尝试超过 3 次仍无法解决时，停止处理该问题，在此记录尝试、证据、影响和后续要求，然后继续其他事项。当前尚无新增达到该阈值的问题。
