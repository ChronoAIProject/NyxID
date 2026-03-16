# NyxID Mobile (Real API Only)

NyxID 移动端（React Native + Expo）。  
本项目已去除内嵌 mock 逻辑，`mobile` 内只保留真实 API 调用代码。

## 目录建议

建议保持同级目录：

- `/backend`
- `/frontend`
- `/mobile`

## 快速开始

```bash
cd mobile
npm install
npm run start
```

原生运行：

```bash
npm run ios
npm run android
```

**Android 本地调试**（环境、模拟器/真机、API 地址）：见 [docs/ANDROID_DEBUG.md](docs/ANDROID_DEBUG.md)。

## 环境变量

`.env` / `.env.example`：

```env
EXPO_PUBLIC_API_BASE_URL=http://localhost:3001/api/v1
EXPO_PUBLIC_IOS_BUNDLE_ID=fun.chrono-ai.nyxid
```

## 当前实现

- 登录：`POST /auth/login`（邮箱密码）
- Challenges 列表：`GET /approvals/requests?status=pending`
- Challenge 详情：`GET /approvals/requests/{id}`
- Challenge 决策：`POST /approvals/requests/{id}/decide`
- Approvals：`GET /approvals/grants`、`DELETE /approvals/grants/{id}`
- Push 设备注册：`POST /notifications/devices`（使用原生 `apns/fcm` token）
- 账号删除：`DELETE /users/me`

## 深链与推送

- 深链协议：`nyxid://challenge/{challenge_id}`
- 路由目标：`ChallengeMinimal`
- 支持 payload 字段：`deeplink`、`url`、`challenge_id`、`challengeId`

## 会话

- Access token 使用 `SecureStore` 持久化
- App 冷启动恢复会话后自动进入 `Dashboard` 或 `Auth`

## 关键文件

- `src/lib/api/mobileApi.ts`
- `src/lib/api/http.ts`
- `src/features/auth/AuthSessionContext.tsx`
- `src/lib/auth/sessionStore.ts`
- `src/app/linking.ts`
