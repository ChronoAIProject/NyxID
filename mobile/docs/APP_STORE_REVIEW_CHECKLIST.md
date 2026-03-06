# App Store 提审风险检查清单

提审前自检用。每一项标注：✅ 已满足 / ⚠️ 需确认 / ❌ 需修复。

---

## 1. 账号与数据（Guideline 5.1.1）

| 项 | 状态 | 说明 |
|----|------|------|
| 应用内提供账号删除 | ✅ | Account & Security → Danger Zone → Delete Account，二次确认，调用 `DELETE /users/me` |
| 删除后数据清除说明 | ✅ | 隐私政策 §8、§9 与条款 §9 写明永久删除、不可恢复 |
| 删除为真删除、重登为新账号 | ✅ | 后端硬删除 User 文档；同一社交身份再登录会**重新注册**为全新账号（新 UUID），旧数据不恢复，符合 5.1.1 |
| 审核员测试账号 | ⚠️ | 若审核员删除你提供的测试账号，同一身份再登录会变成新账号。建议在「审核备注」中写：如需测试删除请先新建一个账号再删，或提供两个测试账号 |

---

## 2. 隐私与合规

| 项 | 状态 | 说明 |
|----|------|------|
| 隐私政策 | ✅ | 应用内完整 Privacy Policy 页面，登录前可点「Privacy」进入 |
| 服务条款 | ✅ | 应用内完整 Terms of Service，登录前可点「Terms」进入 |
| 登录前可见条款/隐私 | ✅ | Auth 页底部 "By continuing, you agree to Terms and Privacy" 可点 |
| 儿童隐私 | ✅ | 隐私政策 §11 声明不面向 16 岁以下 |
| 导出合规 | ✅ | `ITSAppUsesNonExemptEncryption` = false（Info.plist） |
| 隐私清单 PrivacyInfo.xcprivacy | ✅ | 已配置 API 原因码；NSPrivacyTracking=false；NSPrivacyCollectedDataTypes 当前为空 |
| App Store Connect 隐私营养标签 | ⚠️ | 需与隐私政策一致：若在 Connect 中声明「收集的数据类型」，请与 App 内隐私政策（邮箱、设备标识、使用数据等）一致；不追踪已正确 |

---

## 3. Sign in with Apple（Guideline 4.8）

| 项 | 状态 | 说明 |
|----|------|------|
| 提供 Apple 登录 | ✅ | Auth 页有 "Continue with Apple"（与 Google/GitHub 并列） |
| 实现方式 | ✅ | 通过后端 OAuth + `WebBrowser.openAuthSessionAsync`，无需原生 entitlement |
| 条款/隐私中说明 | ✅ | Terms §4、Privacy §4 明确写清 Sign in with Apple 数据使用 |

---

## 4. 权限与能力

| 项 | 状态 | 说明 |
|----|------|------|
| 推送通知 | ✅ | UIBackgroundModes remote-notification；expo-notifications 请求权限，无额外 plist 文案要求 |
| 相机/相册/麦克风/定位 | ✅ | 未使用，无需声明 |
| ATT（跟踪） | ✅ | 未做跨应用跟踪，隐私政策 §10 已说明，NSPrivacyTracking=false |
| 网络 | ✅ | NSAllowsArbitraryLoads=false，NSAllowsLocalNetworking=true（开发/本地调试用） |

---

## 5. 技术配置

| 项 | 状态 | 说明 |
|----|------|------|
| 提审/生产 API | ✅ | `build:ios:testflight` 使用 `EXPO_PUBLIC_API_BASE_URL=https://nyx-api.chrono-ai.fun/api/v1`，审核环境可访问 |
| URL Scheme | ✅ | `nyxid`，用于 OAuth 回调 |
| Associated Domains | ✅ | `applinks:nyx-api.chrono-ai.fun`（entitlements） |
| 最低系统版本 | ✅ | LSMinimumSystemVersion 12.0 |

---

## 6. 常见拒审点自检

| 风险 | 建议 |
|------|------|
| 审核员删掉唯一测试账号 | 在审核备注中说明：请使用提供的账号浏览主要功能；若要测试删除，请先新建账号再删，或注明已提供第二个测试账号 |
| 审核时 API 不可用 | 确保 `nyx-api.chrono-ai.fun` 在审核期间稳定、无 IP 限制或需 VPN |
| 隐私营养标签与 App 不一致 | 在 App Store Connect → App 隐私 中如实填写收集的数据类型（邮箱、设备 ID、使用数据等），与隐私政策一致 |
| 年龄分级 | 若未设 17+，建议与隐私政策「不面向 16 岁以下」一致，选 4+ 或 12+ 并确认无违规内容 |

---

## 7. 建议的审核备注（App Review Information）

可复制到 App Store Connect「备注」中，减少误删测试账号导致的拒审：

```
Test account: [你的测试邮箱/说明]
Password: [若为邮箱密码登录则提供；当前为社交登录则写：Please use "Continue with Google/GitHub/Apple" to sign in.]

Account deletion: Available under Account tab → Account & Security → Danger Zone → Delete Account. Deletion is permanent; if you sign in again with the same provider (Apple/Google/GitHub), a new account will be created and previous data will not be restored. To test deletion, please create a second account first, or use the provided account only for feature review.
```

---

## 8. 结论

- **已满足**：账号删除、条款/隐私、Sign in with Apple、推送、导出合规、隐私清单、生产 API 配置。
- **提审前务必确认**：审核备注中说明测试账号与删除测试方式；App Store Connect 中隐私营养标签与隐私政策一致；`nyx-api.chrono-ai.fun` 在审核期间可访问。

完成上述 ⚠️ 项后，按当前实现无明显硬性拒审风险；具体通过仍以 Apple 当次审核为准。
