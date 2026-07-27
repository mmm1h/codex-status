<div align="center">

# CodexStatus

**在 Windows 托盘里一眼看清 Codex 周剩余额度。**

[English](README.md) · [下载](https://github.com/mmm1h/codex-status/releases/latest) · [反馈问题](https://github.com/mmm1h/codex-status/issues)

</div>

| 浅色 | 深色 |
|:--:|:--:|
| ![CodexStatus 浅色额度卡片](assets/screenshots/codexstatus-light.png) | ![CodexStatus 深色额度卡片](assets/screenshots/codexstatus-dark.png) |

CodexStatus 是一个小巧的原生 Windows 工具。通知区域图标本身就是 `0–100` 的剩余额度数字；暂时没有可信数据时显示 `--`。左键点击可查看重置倒计时、用量节奏、可选的 5 小时窗口、套餐和刷新状态。

## 主要特点

- 在标准系统托盘图标内直接绘制周剩余额度。
- 透明背景、随任务栏明暗切换的 Segoe UI 数字；底部仅用一条细线标记绿色（≥50%）、琥珀色（20–49%）、红色（<20%）或缓存状态。
- 使用 Direct2D + DirectWrite 绘制原生圆角卡片，具备 ClearType 字体、克制的数据可视化，并适配浅色、深色、高对比度和多显示器 DPI。
- 可从托盘菜单选择跟随系统、浅色或深色界面主题。
- 每天静默检查经过校验的 GitHub Release；有新版时自动替换并重新启动。
- 只使用官方 Codex app-server RPC `account/rateLimits/read`，不读取 Token，不访问私有接口。
- 纯 Win32 事件驱动；没有 Electron、WebView、WPF、WinUI、本地 HTTP 服务或常驻异步运行时。
- 默认 5 分钟刷新，支持手动刷新、失败退避、安全缓存过期和可选低额度提醒。
- 周额度与 5 小时额度可分别设置阈值，并支持预计提前耗尽提醒、额度恢复提醒和测试通知。
- 托盘可显示周额度、5 小时额度或两者较低值；没有 5 小时窗口时详情卡片会自动收起该项。
- 可选读取 OpenAI 官方状态页、复制状态/诊断信息、固定详情卡片，以及启用 `Ctrl+Alt+Q` 全局快捷键。
- Windows 锁屏或睡眠时暂停额度与服务状态读取，恢复后只补一次最新数据。
- 单实例、Explorer 重启恢复、多屏定位和开机启动。
- 根据 Windows 自动选择英文或简体中文。

## 安装

需要 Windows 10/11 x64，并已安装且登录 [Codex CLI 或 Codex 应用](https://developers.openai.com/codex/cli/)。

1. 从 [Releases](https://github.com/mmm1h/codex-status/releases/latest) 下载当前用户安装包。
2. 运行安装程序。默认安装到 `%LOCALAPPDATA%\Programs\CodexStatus`，并默认启用开机启动。
3. 如果 Windows 把新图标放进折叠区，请打开折叠区，把 CodexStatus 拖到可见托盘。图标是否常显由 Windows 和用户控制，应用无法强制固定。

当前安装包尚未代码签名，因此 Microsoft Defender SmartScreen 可能提示“无法识别的应用”。每个 Release 都提供 SHA-256 校验文件。便携 ZIP 默认不会修改开机启动，可从右键菜单自行开启。

## 使用

- **左键：** 打开或关闭额度卡片。
- **右键：** 刷新、选择托盘指标、配置周/5 小时/节奏/恢复提醒、切换主题、固定卡片、复制状态或诊断、查看 OpenAI 状态、启用 `Ctrl+Alt+Q`、管理开机启动或退出。
- **托盘数字：** 默认显示周剩余，也可改为 5 小时额度或两者较低值，均四舍五入到整数。
- **额度条标记：** 对比当前周期的“额度剩余”和“时间剩余”，预计会在重置前耗尽时给出明确提示。

每次刷新会短暂启动本机 `codex app-server`，完成 `initialize → account/read → account/rateLimits/read` 后，使用 Windows Job Object 关闭整个子进程树。周窗口优先精确匹配 10,080 分钟；否则只接受 6–8 天窗口，绝不会把短窗口误标成周额度。

## 隐私

CodexStatus 不读取或保存 OAuth Token、邮箱、项目内容、提示词和 app-server 原始响应，也不收集遥测。服务检查只读取 `status.openai.com` 的公开摘要，不发送任何凭据，最多每 15 分钟一次，并可从托盘菜单关闭。自动更新最多每天读取一次 `api.github.com` 的公开最新 Release 元数据；只有存在更高的稳定版本时才下载程序，并且必须通过 GitHub 发布的 SHA-256 摘要校验后才会替换当前程序。

`%LOCALAPPDATA%\CodexStatus` 下只有两个文件：

- `settings.json`：刷新间隔、界面语言、主题、托盘指标、提醒/快捷键/固定选项、首次引导、最近一次成功更新检查和提醒去重状态。
- `snapshot.json`：最近一次经过解析的非敏感额度快照；一旦跨过重置时间立即失效。

普通版本不写日志。只有显式启用 Cargo `diagnostics` 特性时，才记录生命周期阶段和过滤后的错误摘要。

## 性能

在 Windows 11 24H2 x64 上，对本地 v0.4.0 x64 Release 版收起卡片后的常驻状态连续采样 120.77 秒：

| 状态 | CodexStatus 工作集 | CPU | 子进程 |
|---|---:|---:|---:|
| 收起额度卡片后空闲 | 平均 3.56 MB / 峰值 3.86 MB | 实测 0.0% | 0 |
| 刷新中 | 托盘主进程 <15 MB | 短暂活动 | 1 个临时 `codex app-server` 进程树 |

采样结束时仅有 2 个线程，句柄数少于开始值，GDI 与 USER 对象数均未增长，也没有子进程。Direct2D 与 DirectWrite 只在卡片可见时加载；关闭卡片后立即释放绘图对象，并在稍后主动回收工作集。app-server 是 Codex 本身，刷新时会有更大的瞬时占用；完成两个账户查询后立刻退出，不属于常驻托盘进程。如果 Direct2D 初始化失败，上一版 GDI 绘制路径仍会自动兜底。

## 构建

正式发布目标是稳定版 Rust 的 `x86_64-pc-windows-msvc`：

```powershell
cargo fmt --check
cargo clippy --all-targets -- -D warnings
cargo test --all-targets
cargo build --release --locked
```

GitHub Actions 会为版本标签构建便携 ZIP 和 Inno Setup 安装包。本地也可使用 gnullvm 开发工具链；此时 llvm-mingw 的 `libunwind.dll` 只是本地开发依赖，正式 MSVC Release 是单文件程序。

## 首版边界

本项目不做私有任务栏注入、多供应商、Token/成本历史或本地服务。Windows 没有受支持的接口可强制托盘图标常显，因此固定图标始终由用户决定。

## 致谢

交互和信息层级参考了 [CodexBar](https://github.com/steipete/CodexBar)、[TaskbarQuota](https://github.com/zioder/TaskbarQuota)、[CodexQuotaTaskbar](https://github.com/zHysie/CodexQuotaTaskbar)、[codex-win-widget](https://github.com/Mauriciog87/codex-win-widget) 和 [Claude & Codex Battery](https://github.com/dennykim123/claude-codex-battery)；紧凑浮层也借鉴了 [Windows 应用设计指南](https://learn.microsoft.com/windows/apps/design/)、[Twinkle Tray](https://github.com/xanderfrangos/twinkle-tray) 与 [EarTrumpet](https://github.com/File-New-Project/EarTrumpet) 的交互思路。本项目独立实现，没有复制这些项目的源代码。

额度通信遵循官方 [Codex app-server 文档](https://learn.chatgpt.com/docs/app-server#6-rate-limits-chatgpt)，通知区域行为遵循 [Microsoft 指南](https://learn.microsoft.com/windows/win32/uxguide/winenv-notification)。

## 许可证

[MIT](LICENSE)
