<p align="center">
  <img src="design/brand/mark.svg" width="80" alt="AppHost">
</p>

<h1 align="center">AppHost</h1>

<p align="center">把本地项目变成随手可开的应用。</p>
<p align="center"><strong>macOS Alpha 1 · v0.1.0-alpha.1 · Apple Silicon</strong></p>

AppHost 是面向本地开发项目的桌面启动器。把已有项目加入磁贴，确认启动命令后，就可以启动、停止、查看日志和处理失败，不必反复寻找终端窗口。

它支持一个应用包含多个服务，例如 Python 后端与 Vite 前端；也支持没有浏览器入口的后台服务。项目继续使用自己的代码、依赖和运行环境，AppHost 负责登记和运行管理。

**这是 macOS 的早期试用版，不是稳定版，也不代表 M1 已全部验收。** 本次提供 Apple Silicon（arm64）安装包；Intel Mac、Windows 和 Linux 桌面运行不在本次发布范围内。

[下载 Alpha 1](https://github.com/anweat/AppHost/releases/tag/v0.1.0-alpha.1) · [反馈问题](https://github.com/anweat/AppHost/issues) · [Agent 接入](#agent-接入)

## 安装

1. 在 Release 页面下载 `AppHost-v0.1.0-alpha.1-macos-arm64.zip`。同页提供 `SHA256SUMS.txt`。
2. 解压，将 `AppHost.app` 放入 `~/Applications` 或 `/Applications`，再打开应用。
3. 首次设置时明确选择是否开启登录自启；之后可以在设置中修改。

本包使用本地 ad-hoc 签名，**尚未使用 Apple Developer ID 签名或公证**。macOS 可能阻止首次打开。确认来源与校验值后，可在系统「隐私与安全性」中使用系统提供的打开选项；不需要关闭系统安全保护。不希望使用未公证安装包时，可以按下文从源码构建。

可选的校验方式：把 ZIP 和 `SHA256SUMS.txt` 放在同一目录，运行：

```sh
shasum -a 256 -c SHA256SUMS.txt
```

## 第一个应用

1. **准备项目**：确保所需 Node、Python 等运行时和项目依赖已经安装。AppHost 不会自动执行 `npm install`、`uv sync` 等安装命令。
2. **点击「＋」添加应用**：选择项目目录，查看识别到的候选与启动声明。需要时可以选择外置清单。
3. **确认执行内容**：检查命令、工作目录、端口和浏览器入口，确认后加入磁贴。已有 `apphost.toml` 的项目也走明确的确认流程。
4. **点击磁贴启动**：后台尚未运行时，显式启动会拉起后台。运行后再点磁贴，有入口的应用打开浏览器，无入口的服务打开日志。
5. **失败时就地处理**：查看一句话原因、证据和主要操作；需要时进入详情查看日志、运行环境或编辑声明。

若提示找不到 `npm`、`node` 等命令，到「管理 → 运行环境」检查共享 PATH。可以手动编辑，或明确选择从登录 shell 采集；采集会执行该 shell 的初始化脚本。Python 虚拟环境也可以直接在声明中使用 `.venv/bin/python`。

项目搬家后，可以在「应用管理」重新关联路径；解除登记保留项目文件。关闭 AppHost 窗口不会停止已运行的服务。**「全部停止」同时暂停浏览器自动唤醒**，需要继续使用自动唤醒时，点击「恢复唤醒」。

## 磁贴与搜索

- 磁贴保持固定顺序，支持自定义分组、文件夹折叠、组内全部启动／停止，以及可关闭的最近使用组。
- 运行时填充项目色；失败与待确认使用角标。多服务应用用图标外缘的圆角方形分段框表达各服务状态。
- 右键或 `Shift+F10` 打开操作菜单；悬停显示详情入口。
- 点击搜索框、按 `⌘K`，或在画布直接输入即可搜索。建议列表可以排序，画布只临时过滤，不改变固定位置。

| 输入 | 用途 |
| --- | --- |
| `notes` | 查找应用或分组 |
| `is:running` / `is:运行` | 筛选运行中的应用 |
| `is:failed` / `is:失败` | 筛选失败或源码缺失的应用 |
| `in:工具` / `组:工具` | 筛选分组 |
| `is:运行 in:工具` | 组合条件 |
| `日志 notes` / `停止 notes` / `重启 notes` | 查找应用操作 |
| `> 操作记录` / `> 环境` / `> 设置` | 查找 AppHost 功能 |

使用 `↑` / `↓` 选择建议，回车执行，`Shift+回车` 执行应用行的备选操作。`Esc` 先清除查询，再关闭面板。设置页可以调整默认回车行为、路径匹配和建议操作；显式 `>` 始终搜索操作。

## 当前能力

- **登记与声明**：项目扫描、批量选择、桌面确认、声明编辑及 diff、外置清单、切换来源、重新关联路径与解除登记。
- **运行管理**：单／多服务启动、停止、重启、端口预检、就绪判断、后台恢复，以及操作记录。
- **失败处理**：分类建议、端口占用证据、实际监听端口、上次成功以来的变化、失败历史与诊断导出预览。
- **日志与环境**：独立日志采集、日志查看／跟随、终端入口、共享 PATH，以及可修改的日志保留设置。
- **浏览器入口**：本地稳定地址与受限的自动唤醒规则。默认网关端口为 `18480`，可显式修改；占用时报告冲突，不自动换端口。

AppHost 不提供依赖安装、通用开发环境管理、容器编排或远程部署。启动声明中的命令拥有当前用户的权限，请只确认自己信任的项目和命令。

## 启动声明示例

项目根目录中的 `apphost.toml` 描述要执行的命令。以下示例适用于已安装依赖、已有 `npm run dev` 脚本的 Vite 项目：

```toml
schema_version = 1
project_id = "e72b59f7-664c-444e-864e-1d5ec476ea42"
name = "Notes"
open = { service = "web", path = "/" }

[[services]]
name = "web"
cwd = "."
command = ["npm", "run", "dev", "--", "--host", "127.0.0.1", "--port", "5173", "--strictPort"]
port = 5173
startup_timeout_ms = 30000
```

不同逻辑项目应使用不同的 `project_id`；同一项目的多个安装由 AppHost 分别登记。命令使用参数数组，不隐式经过 shell。单个明确声明的端口可以作为默认入口；多端口应用应使用 `open` 指定入口。没有端口的服务也可以登记运行。

完整的单服务、多服务示例及确认规则见 [Agent 清单指南](agent/skills/manifest.md)。

## CLI 与 Agent 接入

桌面包内包含同版本 CLI，不会自动修改系统 PATH。例如安装到 `~/Applications` 后：

```sh
APPHOST_CLI="$HOME/Applications/AppHost.app/Contents/MacOS/apphost"
"$APPHOST_CLI" --version
"$APPHOST_CLI" list --redacted
"$APPHOST_CLI" status
"$APPHOST_CLI" agent-guide
```

CLI 的参数以 `--help` 为准；不是每条命令都接受 `--json`。向 Agent 提供登记信息时使用 `list --redacted`，避免普通 `list` 中的已确认环境变量原值进入上下文。

Agent 交付物：

- [生成与检查启动声明](agent/skills/manifest.md)
- [失败排障与诊断导出](agent/skills/troubleshoot.md)
- [项目提示词片段](agent/project-prompt.md)

`apphost agent-guide` 提供随二进制发布的离线指南。Agent 应检查静态声明和 AppHost 的受控运行环境，并将执行内容交给用户确认；不能因为扫描或校验成功就自行安装依赖、确认声明或启动项目。

## 数据与升级

macOS 默认数据目录为 `~/Library/Application Support/AppHost`，保存登记、运行记录、日志、失败现场与设置。项目源码留在原目录。普通日志缺省保留 7 天／每安装 100 MiB，可在设置中调整；失败现场采用独立保留规则。

Alpha 阶段升级前建议保留数据目录备份。首次登记写入会将旧 Registry v0 备份并升级为 v1；旧 M0 程序不能直接读取升级后的登记表。更新整个桌面包时，先停止服务并退出相应后台，再替换应用，避免同时混用不同版本的后台和采集器。

## Alpha 1 的限制

- 本次安装包仅面向 **macOS Apple Silicon**。当前本机验证环境为 macOS 27；其他系统版本、Intel Mac 仍需兼容性验证。
- Windows 已有 CLI 登记与平台边界工作，但真实服务生命周期、采集器、浏览器和终端入口尚未完成；不发布 Windows 桌面安装包。
- 一周持续使用、跨浏览器自动唤醒矩阵、失败可理解率和真机中文输入法首字合成仍待完整验收。稳定入口的部分场景已实测，不能据此承诺所有浏览器行为一致。
- 分组移动／解散后的撤销入口尚未补齐。
- 运行后的端口丢失有固定 10 秒宽限。更长的重编译可能被标为失败，端口恢复后仍需显式操作；目前不提供逐服务覆盖。
- 未配置自动更新，也尚未完成 Developer ID 签名与公证。

反馈时请提供系统版本、芯片类型、复现步骤与错误码。诊断导出应先预览，移除不希望公开的路径、环境信息和日志内容；不要直接上传整个数据目录。

## 从源码构建

需要 Rust stable（支持 edition 2024）、Node.js 24 与 npm，以及 macOS Xcode Command Line Tools。

```sh
git clone https://github.com/anweat/AppHost.git
cd AppHost
npm --prefix desktop ci
npm --prefix desktop run tauri -- build --bundles app
```

构建前会生成并打包对应目标的 Rust CLI。应用产物位于：

```text
desktop/src-tauri/target/release/bundle/macos/AppHost.app
```

开发窗口与验证命令：

```sh
npm --prefix desktop run tauri -- dev

cargo fmt --all -- --check
cargo clippy --locked --all-targets -- -D warnings
cargo test --locked --all-targets
npm --prefix desktop test
npm --prefix desktop run build
cargo test --locked --manifest-path desktop/src-tauri/Cargo.toml
```

仅需 CLI 时运行 `cargo build --locked --release`。Windows CI 编译和测试通用核心及边界，不代表 Windows 实际托管能力已经完成。

## 设计文档

- [总体边界与阶段范围](AppHost%20v0.2：产品边界与阶段范围冻结稿.md)
- [M1 阶段设计](AppHost%20M1：冻结稿.md)
- [M1 接口设计](AppHost%20M1：接口设计.md)
- [M0 接口设计](AppHost%20M0：接口设计.md)
- [交互原型](design/m1-ui/index.html)

设计文档描述目标和接口约定，可能包含尚待验收的范围；本次发布能力与限制以本 README 和 Alpha 1 Release 说明为准。
