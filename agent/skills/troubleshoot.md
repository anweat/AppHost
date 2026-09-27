# AppHost 故障排查

先定位安装包内 CLI 并运行 agent-guide（默认 JSON，入口说明见 manifest 指引）。以下 APPHOST_CLI 是当前 CLI 的绝对路径，`<...>` 必须替换；独立 data-dir 必须始终一致。只读命令不隐式启动 App，离线 status 是历史快照，不能当作实时状态。

## 取得身份与事实

```sh
APPHOST_ACTOR=agent "$APPHOST_CLI" list --redacted
APPHOST_ACTOR=agent "$APPHOST_CLI" status "<installation_id>"
APPHOST_ACTOR=agent "$APPHOST_CLI" history "<installation_id>"
APPHOST_ACTOR=agent "$APPHOST_CLI" failure "<installation_id>" --json
APPHOST_ACTOR=agent "$APPHOST_CLI" url "<installation_id>"
```

installation_id 来自脱敏登记列表或用户选择的应用；不要把 project_id 当作安装 ID。status.instances 中取该安装的 instance_id；没有运行记录就不虚构实例。logs 必须传两个 ID：

```sh
APPHOST_ACTOR=agent "$APPHOST_CLI" logs "<installation_id>" "<instance_id>" --limit 65536
APPHOST_ACTOR=agent "$APPHOST_CLI" failure "<installation_id>" --failure-id "<failure_id>" --json
```

failure_id 来自 failure.records 留存引用或 history.last_failure；历史实例日志只在仍有归档时可读。NOT_FOUND 可能表示尚无保存现场或已过期，不是“本次操作没有错误”。REVIEW_REQUIRED、BUSY、STALE_LAUNCH、RESTART_REQUIRED、RUNTIME_LIMIT、BACKEND_MISSING 或握手超时等前置/控制错误可能不写现场：使用本次回执的 code/message，不拿更早的失败记录解释。超时也不证明操作未接受，不自动重发写操作。

## 解释与建议

按 failure 响应顶层 advice 的 code、level、title、impact、evidence、actions、clears_when 解释原因、影响和唯一主要操作。连续失败可能改变建议；目录中没有 node_modules 或 .venv 只是提示。版本未观测时不要猜测版本不匹配；包管理器不明确时不要自行选择。声明修改或 PATH 问题可按 manifest 指引做两层只读校验，静态通过不代表启动通过。

建议依赖命令只供用户复制；不要自动安装依赖、执行服务、停止外部占用进程，或主动 start/stop/restart/stop-all。受管占用者也必须由用户确认操作，并由核心重新校验身份。端口释放只意味着可以请用户重试，不自动重试。

```sh
APPHOST_ACTOR=agent "$APPHOST_CLI" failure "<installation_id>" --export
```

导出是 JSON 诊断预览，按应用、命令、环境、类别、证据、输出末尾、上次成功排列。脱敏尽力而为，先让用户检查再分享；不要自动发送给 AI 或第三方。当前确认的命令不等于历史失败命令；导出会注明未包含的信息。

普通 list 为保持 M0 兼容可能包含 env 原值；登记发现用 list --redacted。status、logs、manifest read 和 scan 的本地事实仍可能含项目路径或应用输出；不要原样当作已脱敏导出。不索取密钥，不把项目 env、完整环境变量或模型配置写进反馈。修复完成后请用户显式重试。
