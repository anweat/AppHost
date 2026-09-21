# AppHost M1：接口设计

## 范围与依赖

依据《AppHost M1：冻结稿》，按 M1a0 → M1a → M1b → M1c 推进。M1a0 定义声明、确认和日志采集边界；运行命令复用这些类型，不另建声明模型、工作流引擎或通用插件层。

只保留总设计、阶段设计和接口设计。实现取舍在代码旁解释；边界用例和冒烟场景保存在测试代码，文档不记录开发进度、通过数量或原型迭代。

## Manifest v0 / v1

```toml
schema_version = 1
project_id = "ef42ed4a-b42b-45a6-92fa-069a0e997bd1"
name = "Example"
open = { service = "web", path = "/" }

[[services]]
name = "web"
cwd = "."
command = ["npm", "run", "dev", "--", "--port", "5173"]
port = 5173
startup_timeout_ms = 30000
stop_timeout_ms = 5000
```

沿用 v0 的路径边界、参数数组、身份和未知字段拒绝规则。v0 不允许携带 v1 字段；新增字段不改变原有 v0 JSON/TOML 的输出形状。

| 字段 | v1 约束 |
|---|---|
| open | 可选的 service/path 对象，拒绝未知字段；不是任意 URL |
| open.service | 精确匹配一个服务，该服务必须声明 port |
| open.path | 默认 `/`；单 `/` 起始，不接受 `//`、反斜线、控制字符、query/fragment；路径保持原文，不自动修复 |
| services[].startup_timeout_ms | 可选，1,000–600,000，缺省 30,000 |
| services[].stop_timeout_ms | 可选，1,000–60,000，缺省 5,000 |

`Manifest::open_target()` 返回 `Result<Option<OpenTarget>>`。显式 open 优先；没有 open 时唯一声明端口的服务作为入口，多个服务有端口则返回 `OPEN_TARGET_REQUIRED`，没有端口返回 None。它只解析声明，不读取监听端口、不启动服务、不拼接网关地址。调用方须先完成结构与根目录校验。

`OpenTarget` 仅包含 service、port、path。v0 使用相同的确定性入口规则，且继续沿用原确认摘要。服务同名、无效显式引用和无效路径属于 `VALIDATION_ERROR`。

`Service::startup_timeout_ms()` / `stop_timeout_ms()` 返回执行采用的有效值，不把缺省值回写到文件。

## 确认摘要与历史输入

`Manifest::launch_sha256()` 对结构有效的声明计算摘要：

- v0 完整保留已有 `apphost.launch.v1` 算法和有序 services 序列化，不能把新字段默认值混入历史输入。
- v1 使用 `apphost.launch.v2`；覆盖 schema_version、project_id、完整有序 services 及 open；服务超时先按缺省值规范化，因此省略和显式填写同样的缺省值不会制造变更。open 的省略与显式声明仍是不同确认内容。
- 名称、描述、排版、注释不影响摘要。声明摘要不签名脚本内容、PATH 中的可执行文件内容或依赖版本。

登记确认在 installation 上记录声明版本和确认摘要，完整快照只用于比较；不能由当前清单反推未知旧快照。Registry v0 中的 confirmed_services 仍以 Manifest v0 解释。

Registry v1 的快照使用 `confirmed_manifest` 保存与确认摘要匹配的完整声明；保留 `confirmed_services` 供旧输出调用方读取，但两份内容必须一致。确认上下文记录 project_root 与 manifest_source，不能把其他安装的确认直接移过来。历史 v0 读取不隐式写入；需要写入 v1 声明时，在原 registry 锁中保存权限受限的原始备份并原子发布 v1。未知版本或不匹配快照返回 REGISTRY_INVALID，不重置。

ImportDraft v0 仅允许 v0 声明，保持原写入语义；包含 v1 清单时使用 Draft v1。扫描推断的新清单仍可使用 v0，不强制升级项目；现有 v1 清单必须经过同一 scan → review → import 入口。重复导入、内容复核、过期草稿、部分成功回执延续 M0。relink 成功改变根目录后清除下次启动确认，保留历史快照供比较。

## 清单来源

解析结果包含 `project_root / manifest_source / manifest_path / manifest`。source 为 project 或 external，登记指向唯一生效来源，不建立 overlay 合并。

- 项目内清单：`<project_root>/apphost.toml`。
- 外置清单：`<data>/manifests/<installation_id>.toml`，cwd 仍相对 project_root。
- 切换来源先校验目标，再持锁切换登记；原文件保留，不静默删除或覆盖。
- 普通文件校验延续 `validate PATH`；外置校验显式传项目根目录，不能以清单父目录推导。
- 活动实例上的 detach/relink/来源切换经运行协调层先停止；L0 只负责短锁、校验和持久化，不调用 daemon。

## 日志采集边界

验证方向为每个活动实例一个独立采集进程。由服务写入采集进程持有的管道；daemon 不持有采集生命周期的唯一引用。采集器只写日志及观察索引，不写 Runtime State、不决定重试、不启动或停止服务。零活动 App 时不保留采集进程。

原型只验证输出接收、轮转、观察时间、流结束与父进程退出；它不替代进程身份对账、安装范围留存、失败分类或 GUI。原型使用真实子进程，所有输出在临时目录，测试完成清理。

日志内核使用固定大小字节块，不依赖换行或 UTF-8。原始字节分流保存，观察元数据单独保存：

```text
sequence       采集器内递增序号
observed_ms    观察到字节块的墙钟时间
elapsed_ms     相对采集器启动的单调时间
service_key    受信任的安全服务键（原名不用于文件路径）
stream         stdout | stderr
segment        轮转段编号
offset / length 原始文件中的字节范围
```

这表示观察顺序，不表示两个流的真实因果顺序。服务原名包含路径字符时必须编码或映射，不能直接拼进日志路径。

每段有容量上限；创建新段后按最老段清理，原始文件和对应索引一起清理。缓冲、队列及索引都必须有界；超长行和二进制数据不引发无界分配。普通留存按安装总计 7 天 / 100 MB，失败现场另存，不能仅对单个 stdout 文件应用整个安装的预算。

IO 失败向调用方返回错误，原型不切换到另一种隐藏存储。磁盘满后的服务处理、采集器自身崩溃后的处理由原型证据和产品决定明确后实现，不能静默丢弃且声称完整日志。

## 运行接口约束

运行状态仅 STOPPED / STARTING / RUNNING / STOPPING / FAILED；就绪、退出原因和 management_uncertain 为附加信息。操作 ID 用来关联长请求，不另建持久化任务状态机。

请求采用 JSON Lines：`{version, request_id, method, params}`；响应回显 request_id，成功返回 result，错误返回沿用 M0 的 `{error:{code,message,details}}`。长操作返回 operation_id 后通过状态查询/事件获得结果；断连不取消已接受的操作。事件游标与快照绑定，过期游标要求重新读取快照。

| 方法 | 输入 | 行为 |
|---|---|---|
| start | installation_id | RUNNING 幂等；STARTING 合并；FAILED 仍有存活服务时要求 restart；启动前核验当前声明 |
| stop | installation_id | 可取消 STARTING；退出已证明归属的全部进程后才报告 STOPPED |
| restart | installation_id | 先停止旧实例，成功后重新校验并分配新 instance_id |
| status | installation_id 可选 | 返回当前状态、活动服务与提示；daemon 离线不将历史视为实时 |
| logs | installation_id、instance_id、service、stream、cursor、limit | 有界读取；跟随模式输出 JSON Lines，慢读不阻塞 stop |
| url | installation_id | 只读解析稳定入口，不拉起 daemon 或 App |
| failure | installation_id、failure_id 可选 | 返回结构化失败、证据和现场完整性 |
| stop-all | 无 | 暂停自动唤醒、取消待启动、停止所有活动服务，返回逐项结果 |
| journal | cursor、limit | 有界读取操作日志 |

网关固定端口、浏览器唤醒条件和 Windows Job 生命周期按阶段稿的验证要求确定；这里不先定义兼容回退组合。平台未支持能力明确返回 UNSUPPORTED_PLATFORM。
