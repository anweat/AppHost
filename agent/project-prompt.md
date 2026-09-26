# 可粘贴到项目 AGENTS.md 或 CLAUDE.md 的片段

本项目使用 AppHost 管理本地启动声明。适配或排障前执行 `apphost agent-guide`，遵循其中与当前版本匹配的清单和排障指引。只读诊断优先使用 status、failure --json、logs、url、list --redacted。修改清单后 validate --check-launch 并交由用户确认；依赖命令只供用户复制。不要自行确认声明、安装依赖或主动 start/stop/restart/stop-all。诊断导出先预览脱敏结果，未经用户指令不外发。
