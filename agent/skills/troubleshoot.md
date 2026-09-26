# AppHost 故障排查

先运行 `apphost agent-guide`，使用 `status`、`failure <id> --json`、`logs`、`url` 读取事实。这些读取不隐式启动 App。离线 status 是历史快照，不能当作实时状态。

按 failure.advice 的 code、level、title、evidence、actions、clears_when 解释原因、影响和唯一主要操作。连续失败可能改变建议；目录中没有 node_modules 或 .venv 只是提示。版本未观测时不要猜测版本不匹配；包管理器不明确时不要自行选择。

建议依赖命令只供用户复制；不要自动安装依赖、执行服务、停止外部占用进程，或主动 start/stop/restart/stop-all。受管占用者也必须由用户确认操作，并由核心重新校验身份。

用 `apphost failure <id> --export` 取得按应用、命令、环境、类别、证据、输出末尾、上次成功排列的诊断预览。脱敏尽力而为，先让用户检查再分享；不要自动发送给 AI 或第三方。当前确认的命令并不等于历史失败命令；导出会注明未包含的信息。

普通 list 为保持 M0 兼容可能包含 env 原值；登记发现用 list --redacted。不要索取密钥或把项目 env、完整环境变量写进反馈。修复完成后请用户显式重试。
