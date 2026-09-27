# AppHost 清单适配

只帮助适配清单，不自行确认或启动应用。先定位当前安装的 CLI：如果 PATH 中没有 apphost，macOS 桌面包内的入口是 `<AppHost.app>/Contents/MacOS/apphost`，常见安装位置为 `~/Applications/AppHost.app` 或 `/Applications/AppHost.app`。用实际存在的路径，不猜另一个版本，不修改系统 PATH。以下示例把该路径保存为 APPHOST_CLI，所有路径都加引号：

```sh
APPHOST_CLI="$HOME/Applications/AppHost.app/Contents/MacOS/apphost"
APPHOST_ACTOR=agent "$APPHOST_CLI" agent-guide
```

`agent-guide` 默认输出 JSON，包含 version、manifest、troubleshoot、project_prompt；无需也不接受 `--json`。下文 `<...>` 是应替换的值，不能原样执行。指定独立数据目录时，所有命令使用同一个 `--data-dir`；读取不应创建目录，确需创建测试目录时设为仅当前用户可访问。`APPHOST_ACTOR=agent` 只是行为来源标记，不是鉴权或安全隔离。

## 发现与编写

```sh
APPHOST_ACTOR=agent "$APPHOST_CLI" scan "<项目根目录>"
APPHOST_ACTOR=agent "$APPHOST_CLI" list --redacted
```

从 README、package.json、pyproject.toml 及明确的启动配置确定命令和端口。scan 返回的 draft 只是候选，空 services 或 warnings 不应被当成最终清单；多服务项目可能需要依据 nested_projects 合并为一个明确声明，不靠执行脚本猜测。新项目只生成一次 project_id；已有清单和同一逻辑项目的副本复用它，每个登记路径另有 installation_id。`list --redacted` 是身份投影，省略路径和标签；用清单的 project_id 匹配候选，再由用户核对具体副本，不从脱敏输出猜路径。普通 list 为兼容 M0 可能包含 env 原值，不用于诊断导出。

最小 v1 单服务示例（示例 UUID 仅说明格式，新项目生成自己的 UUID）：

```toml
schema_version = 1
project_id = "ef42ed4a-b42b-45a6-92fa-069a0e997bd1"
name = "Example API"

[[services]]
name = "api"
cwd = "."
command = [".venv/bin/python", "-m", "uvicorn", "backend.main:app", "--host", "127.0.0.1", "--port", "8000"]
port = 8000
```

多服务示例（选择前端作为入口；服务数组不代表启动顺序）：

```toml
schema_version = 1
project_id = "4ae67972-a0d0-4da2-a682-454857684f93"
name = "Example Web App"
open = { service = "web", path = "/" }

[[services]]
name = "api"
cwd = "."
command = [".venv/bin/python", "-m", "uvicorn", "backend.main:app", "--host", "127.0.0.1", "--port", "8000"]
port = 8000

[[services]]
name = "web"
cwd = "web"
command = ["npm", "run", "dev", "--", "--host", "127.0.0.1", "--port", "5173", "--strictPort"]
port = 5173
startup_timeout_ms = 30000
stop_timeout_ms = 5000
```

上述模块名和 npm 参数只适用于对应项目，不能照抄到未知框架。M1 不自动安装依赖；优先引用项目已有运行时。若使用 uv 包装器，须理解它是否可能同步依赖或下载 Python；不能把安装隐含在启动命令里。

字段契约：顶层必需 schema_version、project_id、name、services，description 可选。每个服务必需 name 与非空 command 参数数组；cwd 默认 `.`，必须是项目根内存在的相对目录；env 默认空表，不写密钥；port 是可选的单个 1–65535 整数，不是 ports 数组。v1 可选启动超时 1000–600000 ms、停止超时 1000–60000 ms，默认 30000 / 5000。open 只有 service、path 两项，不包含 port；service 必须引用有 port 的服务，path 默认为 `/`，不接受外部 URL、query 或 fragment。单一声明端口可自动作为入口，多端口需显式 open；无端口表示纯进程服务。旧 v0 可以保留，新增 open/超时必须用 v1。command 不拼接 shell；修改 port 本身不会更改服务实际监听地址，须与命令/项目配置一致。

## 分层校验与交接

```sh
APPHOST_ACTOR=agent "$APPHOST_CLI" validate "<清单>" --project-root "<项目根目录>" --check-launch
APPHOST_ACTOR=agent "$APPHOST_CLI" environment check "<项目根目录>"
```

第一条校验清单结构、项目内 cwd，并提供当前调用环境 PATH/显式 env.PATH 下的可执行定位、已知 npm 脚本等静态警告；**退出 0 或 warnings 为空不证明依赖、Python 模块、业务健康或实际启动成功**。它不运行命令，也不验证 uv 等包装器内部的目标。第二条用 AppHost 共享运行环境检查项目内清单，避免终端能找到 npm 而桌面找不到；不执行服务。外置声明或已登记安装使用 `environment check --installation <installation_id>` 核验当前生效来源和确认，不能用项目里未生效的文件冒充。PATH 不足时解释缺少的命令，引导用户在“运行环境”设置保存 PATH 或主动采集；采集会运行交互式登录 shell 的初始化脚本，不能为了诊断自行执行。

首次接入优先交给用户在桌面“添加应用”选择项目、复核摘要并导入。已登记项目：

```sh
APPHOST_ACTOR=agent "$APPHOST_CLI" manifest read "<installation_id>"
APPHOST_ACTOR=agent "$APPHOST_CLI" manifest review "<installation_id>" --file "<编辑缓冲文件>"
```

展示命令、cwd、env 名称、端口、入口及变化，由用户在桌面“保存并确认启动声明”。若用户使用 CLI，manifest save 需要 review 输出的 base_sha256、revision 和顶层 launch_sha256（传给 --confirm，不是 previous_confirmation.launch_sha256）；不可自行拼旧 token 或代为确认。修复后报告静态校验范围和未验证部分，请用户显式重试；不主动 start、stop、restart、stop-all，也不执行依赖安装。
