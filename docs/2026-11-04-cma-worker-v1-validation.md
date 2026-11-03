# CMA Worker v1 本地验收记录

本记录对应 `codex/cma-worker-v1` 分支的 NyxID 实现。源需求为 `2026-11-03-cma-worker-v1-implementation.md`，界面参考为 `cma-preview` 的 Sandboxes、执行记录与 Desktop 页面。

## 验证结果

| 检查 | 结果与范围 |
| --- | --- |
| Rust 编译与合并回归 | 最终源码完成测试目标编译，合并筛选运行结果为 **244 passed、0 failed、1 ignored**，覆盖 CMA、异步操作、原生工具权限、调用续接、功能开关、catalog overlay、账号工具与指令上下文。此前 `cargo check -p nyxid --bin nyxid-server --tests` 也已通过。 |
| 双账号隔离回归 | 初始筛选器未匹配实际模块路径；用同一份新编译测试程序补跑 `two_accounts_keep_real_credentials_memory_tools_payers_and_web_continuity`，**1 passed、0 failed**。两轮合计 245 个不同用例通过。 |
| MongoDB | 使用隔离的 MongoDB 8.0 副本集 `cmaTest`，监听 `127.0.0.1:27019`。10 个新增事务用例另以 `--nocapture` 复核，**10 passed、0 failed、0 ignored**，输出没有数据库跳过消息；这 10 项已包含在上述 245 项中，不重复计数。事务探针也验证了在一个事务内删除发现记录，并把其部分唯一索引 remote ID 合并到原创建记录；探针数据库已删除。 |
| Vitest | 12 个相关文件累计 140 条通过，覆盖 DTO、HTTP 边界、hooks、Sandbox/Agent 页面、事件游标、Desktop 文案、任务卡、人工协助卡及续接。 |
| Playwright | `npx playwright test e2e/cma-worker.spec.ts`：11 条通过；覆盖 1440 × 1000 和 390 × 844。 |
| 前端生产构建 | `npm run build` 通过，包括 TypeScript、主应用、legal prerender、credential-accept 和 mock 排除检查。保留原有 CSS 与大 chunk 警告，未修改无关构建配置。 |
| ESLint | 变更的 TS/TSX 文件定向检查通过；最终 Desktop 文案和新增浏览器测试文件再次检查通过。 |
| 格式和空白 | `cargo fmt --all --check` 与 `git diff --cached --check` 通过。 |

浏览器测试使用实际应用页面及 HTTP hooks，在现有端口 4611 的测试环境中拦截 API/WS。测试模式的 assistant HTTP mock 仅对 CMA 路径让出控制，由 Playwright 提供固定响应；没有替换生产组件或修改真实后端资源。这些测试证明本地页面和传输边界行为，不证明真实 CMA 事件或 RFB 连接已经互通。

首次默认调试配置的 Rust 测试编译在代码生成阶段占用约 29 GB，超过本机 18 GB 物理内存，已主动中止；这一轮不算测试通过。后续验证采用仓库已有验证记录使用的低内存参数：`CARGO_INCREMENTAL=0 CARGO_BUILD_JOBS=2 CARGO_PROFILE_TEST_DEBUG=0 CARGO_PROFILE_DEV_DEBUG=0`，不修改仓库构建配置或测试范围。

最终运行沿用 CI 的 `RUST_MIN_STACK=8388608`。默认 libtest 栈曾导致栈溢出，该次运行不计入通过结果。最终合并回归的唯一 ignored 项为既有的 `services::resource_request_proof::tests::joined::serve_joined_cma_acceptance_fixture`，它要求独立的 CMA 配对环境和私有 manifest；本次未执行，也不计作通过。

## 后端验证命令与证据

以下为最终合并运行的实际命令，工作目录为仓库根目录：

```bash
RUST_MIN_STACK=8388608 \
CARGO_INCREMENTAL=0 \
CARGO_BUILD_JOBS=2 \
CARGO_PROFILE_TEST_DEBUG=0 \
CARGO_PROFILE_DEV_DEBUG=0 \
NYXID_TEST_DATABASE_URL='mongodb://127.0.0.1:27019/?replicaSet=cmaTest' \
cargo test -p nyxid --bin nyxid-server -- \
  cma \
  async_service_operation_tests \
  assistant_authority_tests \
  assistant_invocations::tests \
  feature_flag_service \
  catalog_spec_registry \
  assistant_account_tools \
  assistant_instruction_context \
  platform_manager_journey_tests::account_parity \
  --test-threads=4
```

结果：`244 passed; 0 failed; 1 ignored; 10851 filtered out`，测试运行 218.91 秒。输出保存在本机 `/tmp/nyxid-cma-worker-v1-final-rust.log`。

双账号用例实际位于 `services::platform_manager_transport::journey_tests::account_parity`。补跑直接使用上述 Cargo 输出的二进制，避免无源码变化的重复编译；后续复现时应使用当次 `Running unittests` 输出的路径：

```bash
RUST_MIN_STACK=8388608 \
NYXID_TEST_DATABASE_URL='mongodb://127.0.0.1:27019/?replicaSet=cmaTest' \
target/debug/deps/nyxid_server-1140cef517551115 \
  two_accounts_keep_real_credentials_memory_tools_payers_and_web_continuity \
  --test-threads=1
```

结果：`1 passed; 0 failed; 0 ignored`，运行 24.57 秒。输出保存在 `/tmp/nyxid-cma-worker-v1-final-account-parity.log`。

同一二进制使用下列筛选器显式复核数据库执行，没有跳过消息：

```bash
RUST_MIN_STACK=8388608 \
NYXID_TEST_DATABASE_URL='mongodb://127.0.0.1:27019/?replicaSet=cmaTest' \
target/debug/deps/nyxid_server-1140cef517551115 \
  erased_owner_and_stale_generation_cannot_write_creation_results \
  no_resource_confirmation_and_slot_release_commit_together \
  sandbox_refresh_recovers_a_known_operation_without_a_chat_or_watch \
  recovery_is_bounded_and_failure_never_proves_no_resource \
  reacquired_parent_local_creation_rejects_old_submission_receipts \
  concurrent_first_creations_occupy_one_slot_and_retries_keep_identity \
  retirement_acknowledgement_and_timeout_keep_slot_until_exact_read_confirms \
  creation_receipt_folds_an_early_discovery_into_the_original_identity \
  existing_external_environment_blocks_creation_even_when_paused \
  live_listing_registers_all_conflicts_and_never_releases_a_missing_resource \
  --nocapture --test-threads=2
```

结果：`10 passed; 0 failed; 0 ignored`，运行 1.25 秒。输出保存在 `/tmp/nyxid-cma-worker-v1-final-mongo.log`。这些本机临时日志不作为仓库交付文件。

## 覆盖的关键行为

- 空页面不创建资源；开关关闭时隐藏入口，并阻止页面发起 CMA 请求。
- 个人账号唯一占位跨 chat、角色、key 和 catalog 连接别名共享；未知、暂停、创建中和退休受理均保留占位。
- 创建记录保留原始请求字节、哈希与幂等身份；独立于 chat/watch 生命周期，使用 generation、submission generation 和擦除栅栏保护恢复与晚到回执。
- 通用创建端点在其 24 小时去重缓存到期前停止不确定重放；已有资源回执只做读取恢复。永久 lifecycle 身份继续保持同 body/key。
- 退休要求输入环境名称，HTTP 202 显示处理中并保留 `1 / 1 used`；观察确认退休后才释放。
- 当前任务绑定精确 Agent/Response；缺少绑定不选择最新任务，身份不匹配不展示输出或建立事件流。共享 Agent 历史明确标注，游标与当前 Response 分离。
- Desktop 从只读模式开始，缺少能力或 Agent 不可用时不建立连接、不创建资源；页面关闭不取消远端任务。
- 已提交 watch 的后续确认不依赖新增 UI 功能开关；每次观察仍检查当前密钥、授权、原始绑定与操作/输入范围。
- CPU/内存读取 Sandbox 指定的 Profile 固定 revision；返回身份不匹配或配置不可读时保留未知。每页最多八个不同配置读取、共两秒预算，不改用最新版本。
- guarded 审批与人工 Done/Cancel 通过既有确认和事件机制续接原线程，不把停止本地跟踪当作远端取消。

## 上线前仍需目标部署验收

以下属于目标 CMA 部署或跨仓库合同，未通过本地测试替代：

1. CMA effective owner 配额为一个 Sandbox，并覆盖 lifecycle、manual、launch 等所有适用创建入口。NyxID 本地占位不能约束直接调用 CMA 的用户。
2. 首次凭据准备后的同 body/key 重试：本地 CMA 源码要求被拒绝的 keyed command 更换 key，而本方案要求保留同一个 key。NyxID 保留原身份，不自动切换为新意图；启用前需要 CMA 合同对齐。
3. 真实身份透传、授权变更、事件重连及 RFB Desktop 的目标版本互通。
4. CMA 可选的 `callback_url` 生产端。NyxID 内部执行查看页及原线程结果回传不依赖该字段。

`assistant:cma-sandboxes` 默认关闭。目标环境的 CMA catalog 可使用已托管的 `/api/v1/catalog-specs/cma/openapi.json`；应在目标部署完成上述验收后按账号启用。

范围排除和重复阻塞记录见 [外部依赖与实施记录](2026-11-04-cma-worker-v1-out-of-scope.md)。本次未部署、未推送分支，也未删除 Remote Machine 或新增自动回收、Agent 池和算力计费。

## 本地交付

交付分支为 `codex/cma-worker-v1`，worktree 位于 `/Users/jasonwang/.codex/worktrees/cma-worker-v1/NyxID-jason`，基于 `2fcc9a9cd`。实现、回归测试与本验收记录作为本地提交保存，保留 worktree 供审阅。源方案和 `cma-preview` 原型副本保留为未跟踪的参考文件，不纳入实现提交。所有本地必需检查已完成；上述目标 CMA 合同和真实连接验收仍是启用前条件。
