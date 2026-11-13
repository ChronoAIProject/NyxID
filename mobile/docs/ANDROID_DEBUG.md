# NyxID Mobile — Android 本地调试

## 1. 环境准备

### 1.1 必需

- **Node** 18+，**pnpm**
- **Java 17**（Expo / React Native 推荐）
- **Android SDK**（通过 [Android Studio](https://developer.android.com/studio) 安装，或仅安装 Command Line Tools）
- **环境变量**（写入 `~/.zshrc` 或 `~/.bashrc`）：

```bash
export ANDROID_HOME=$HOME/Library/Android/sdk   # macOS 常见路径
export PATH=$PATH:$ANDROID_HOME/emulator
export PATH=$PATH:$ANDROID_HOME/platform-tools
```

### 1.2 设备二选一

- **模拟器**：Android Studio → Device Manager 创建 AVD，或命令行 `$ANDROID_HOME/emulator/emulator -avd <AVD_NAME>`
- **真机**：USB 连接，开启「开发者选项」→「USB 调试」，`adb devices` 能看到设备即可

---

## 2. 项目与 API 地址

```bash
cd mobile
pnpm install
```

**API 地址**（`mobile/.env.dev`，因为 `pnpm android` 用 `APP_ENV=dev`）：

- **模拟器**：`DEV_API_BASE_URL=http://10.0.2.2:3001/api/v1`（Android 模拟器里 `10.0.2.2` 指向宿主机）
- **真机**：用本机局域网 IP，例如 `DEV_API_BASE_URL=http://192.168.1.100:3001/api/v1`（先在本机起好 backend）

确保 backend 在对应地址可访问（如 `cargo run` 在 3001 端口）。

> `EXPO_PUBLIC_API_BASE_URL` 直接设置会被 `app.config.ts` 覆盖。请改 `DEV_API_BASE_URL`（或 `PROD_API_BASE_URL` 若运行 `APP_ENV=prod`）。

开发包使用独立的 `DEV_ANDROID_PACKAGE` 时，不要复用生产的 `google-services.json`。如果需要在开发包测试 FCM，把对应 Firebase 应用下载的 JSON 放到本地，并设置 `DEV_GOOGLE_SERVICES_FILE`；配置会在 prebuild 前校验 JSON 中的包名。未设置时，开发构建不会注入生产 Firebase 应用，远程 FCM 不作为本地开发检查项。

---

## 3. 运行与调试

### 3.1 运行 Android（每次按开发 profile 重生成原生目录）

```bash
pnpm android
```

这个命令会先执行 `expo prebuild --platform android --clean`，再编译安装开发包。每次清理重生成可以移除之前生产构建留下的 `google-services.json` 和 Gradle 配置，避免从 `APP_ENV=prod` 切换到 `APP_ENV=dev` 时误用生产 Firebase。首次运行和切换 profile 时较慢属正常。

### 3.2 指定设备

多设备时指定一个：

```bash
pnpm run android -- --device    # 选真机；脚本仍会先 clean prebuild
pnpm run android                # 默认选模拟器或唯一设备
```

### 3.3 仅启动 Metro（不编译原生）

已装过 App、只改 JS/TS 时：

```bash
pnpm start
```

然后在模拟器/真机里打开已安装的 NyxID，或按终端里的 `a` 在已连接设备上打开。

---

## 4. 常见问题

| 现象 | 处理 |
|------|------|
| `ANDROID_HOME` 未设置 | 设置并 `source ~/.zshrc` 后重试 |
| 真机访问不到 API | 用本机局域网 IP，不要用 `localhost`；确认手机和电脑同一网段 |
| 模拟器访问不到 API | 用 `10.0.2.2:3001`，不要用 `localhost` |
| Gradle / androidx 相关报错 | `pnpm build:android` 已自动跑 `patch-android-build-gradle.js`；如需手动可用 `EAS_BUILD_PLATFORM=android node scripts/patch-android-build-gradle.js`（仅在 `android/build.gradle` 存在时生效） |
| `adb devices` 为空 | 真机：换线/换口、确认 USB 调试已开、重插后点「允许」；模拟器：先启动 AVD 再 `pnpm android` |

---

## 5. 小结

1. 装好 Java 17、Android SDK，设好 `ANDROID_HOME`。
2. `mobile/.env.dev` 里按设备类型设 `DEV_API_BASE_URL`（模拟器 `10.0.2.2:3001`，真机本机 IP）。
3. 在项目根起 backend，在 `mobile` 下执行 `pnpm android` 即可本地编译并跑起 Android 调试。
