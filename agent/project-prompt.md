# 可粘贴到项目 AGENTS.md 或 CLAUDE.md 的片段

本项目使用 AppHost 管理本地启动声明。适配或排障前运行当前安装的 `apphost agent-guide`（默认 JSON）。若 apphost 不在 PATH，macOS CLI 位于实际安装的 `<AppHost.app>/Contents/MacOS/apphost`，常见位置为 `~/Applications/AppHost.app` 或 `/Applications/AppHost.app`；使用带引号的绝对路径，不假设已装全局 CLI。遵循随包 manifest 和 troubleshoot 指引的字段、示例和 ID 获取流程。

只读诊断优先使用 status、history、failure --json、logs、url、list --redacted；logs 需要 installation_id 和 instance_id。修改清单后先 validate --check-launch，再 environment check 检查 AppHost 的运行环境；静态通过不代表依赖齐全或启动成功。依赖命令只供用户复制，不自行采集 shell PATH。不代替用户确认声明、安装依赖或主动 start/stop/restart/stop-all。没有匹配失败现场时报告本次操作 code/message，不借用旧现场；超时不自动重发写操作。诊断导出先预览脱敏结果，未经用户指令不外发。
