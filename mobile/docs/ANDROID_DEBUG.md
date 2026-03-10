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

**API 地址**（`mobile/.env`）：

- **模拟器**：`EXPO_PUBLIC_API_BASE_URL=http://10.0.2.2:3001/api/v1`（Android 模拟器里 `10.0.2.2` 指向宿主机）
- **真机**：用本机局域网 IP，例如 `EXPO_PUBLIC_API_BASE_URL=http://192.168.1.100:3001/api/v1`（先在本机起好 backend）

确保 backend 在对应地址可访问（如 `cargo run` 在 3001 端口）。

---

## 3. 运行与调试

### 3.1 首次运行（会生成 `android/`）

```bash
pnpm android
```

等价于 `expo run:android`：若没有 `android/` 目录会先 **prebuild** 再编译安装。首次较慢属正常。

### 3.2 指定设备

多设备时指定一个：

```bash
npx expo run:android --device   # 选真机
npx expo run:android            # 默认选模拟器或唯一设备
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
| Gradle / androidx 相关报错 | 若 prebuild 后出现，可手动跑一次 patch：`EAS_BUILD_PLATFORM=android node scripts/patch-android-build-gradle.js`（仅当脚本检测到 `android/build.gradle` 时生效） |
| `adb devices` 为空 | 真机：换线/换口、确认 USB 调试已开、重插后点「允许」；模拟器：先启动 AVD 再 `pnpm android` |

---

## 5. 小结

1. 装好 Java 17、Android SDK，设好 `ANDROID_HOME`。
2. `mobile/.env` 里按设备类型设 `EXPO_PUBLIC_API_BASE_URL`（模拟器 `10.0.2.2:3001`，真机本机 IP）。
3. 在项目根起 backend，在 `mobile` 下执行 `pnpm android` 即可本地编译并跑起 Android 调试。
