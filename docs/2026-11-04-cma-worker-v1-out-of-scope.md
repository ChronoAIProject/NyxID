# CMA Worker v1：外部依赖与实施记录

本文件跟踪实现方案的边界和实际验收条件，不把尚未完成的工作计作已完成。

## 本仓库之外的工作

- CMA effective owner Sandbox quota = 1：方案 §4.1、§7.1。需要目标 CMA 部署验证 lifecycle、manual、launch 等创建入口共用 owner 配额；NyxID 本地原子占位不能约束用户直接在 CMA 创建资源。本任务不修改 CMA 全局配额，避免影响其他账号。
- CMA `callback_url` 生产端：方案 §7.3。需要 CMA 发布；NyxID 内部执行页不能依赖此字段。现有 watch 仍负责向原 chat 回传结果。
- Remote Machine 删除、Agent 池/自动回收、Sandbox 级 Desktop/exec/files、独立云端算力计费：方案 §1 明确排除，不在本次实现。

## 已发现的合同差异

- **首次登录后的幂等重试**：方案 §4.2、§6 要求保留原 body 与同一个 Idempotency-Key。本机 CMA 源码 `api/crates/layer1-public/src/rest/problems.rs` 的 `SANDBOX_CREDENTIAL_SETUP_DETAIL` 则要求对已被拒绝的 keyed command 使用新的 Idempotency-Key。NyxID 不自动换 key 或悄悄新建资源；同 key 恢复的端到端验收需要 CMA 合同对齐。在目标版本确认前不能宣称这一场景已验收。
- **桌面协议**：CMA Agent Desktop 是 RFB 二进制 WS，NyxID Machine 桌面是另一种协议。CMA 使用固定版本 noVNC 与独立授权中继，并沿用 first-party 身份、精确 Origin、期限和资源复查规则。

## 重复阻塞规则

按用户要求，同一问题反复尝试超过 3 次仍无法解决时，停止处理该问题，在此记录尝试、证据、影响和后续要求，然后继续其他事项。当前尚无新增达到该阈值的问题。
