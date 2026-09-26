# AppHost 清单适配

先运行 `apphost agent-guide` 获取当前二进制版本对应的指引。只帮助适配清单，不自行启动应用。

1. 用 `apphost scan <目录>` 发现候选；使用 `apphost list --redacted` 查看登记身份，`apphost status` 查看运行状态。普通 `list` 为兼容 M0 可能包含 env 原值，不要把它用于诊断导出。
2. 新项目只生成一次 project_id；同一逻辑项目的复制目录保持相同 project_id，每个安装有独立 installation_id。不要为每次启动重建 ID。
3. command 使用 argv 数组，不拼接 shell 命令。cwd 必须相对且在项目根目录内。env 不写密钥。端口必须来自明确声明；唯一声明端口可作浏览器入口，多端口需显式 open。
4. 编辑 apphost.toml 或用户提供的外置清单缓冲区后运行 `apphost validate <清单> --project-root <根目录> --check-launch`；不执行依赖安装或启动脚本来猜测配置。
5. 已登记项目用 `apphost manifest read <id>` 读取，`manifest review <id> --file <清单>` 查看差异，然后请用户确认并执行 manifest save。首次登记也由用户 review/import 确认。Agent 不代替用户确认启动声明。
6. 修复后报告改动和验证结果，请用户显式重试；不要主动 start、stop、restart 或 stop-all。

APPHOST_ACTOR=agent 只是行为来源标记，不是鉴权或安全隔离。
