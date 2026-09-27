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

Registry v1 的快照使用 `confirmed_manifest` 保存与确认摘要匹配的完整声明；保留 `confirmed_services` 供旧输出调用方读取，但两份内容必须一致。确认上下文记录 project_root 与 manifest_source，不能把其他安装的确认直接移过来。历史 v0 读取不隐式写入；首次登记写入（包括 import、label、detach、retention、relink）时，在原 registry 锁中保存权限受限的原始备份并原子发布 v1。新建登记表直接采用 v1；Manifest 和 Draft v0 继续作为兼容输入。未知版本或不匹配快照返回 REGISTRY_INVALID，不重置。若导入未完成、磁盘登记表仍为 v0，下一次迁移可用当前 v0 原始字节刷新此前暂存的 v0 备份；已经发布 v1 后备份不再改写。无法识别为 v0 的既有备份返回 CONFLICT，提示保留或移走该文件后重试。

ImportDraft v0 仅允许 v0 声明，保持原写入语义；包含 v1 清单时使用 Draft v1。扫描推断的新清单仍可使用 v0，不强制升级项目；现有 v1 清单必须经过同一 scan → review → import 入口。重复导入、内容复核、过期草稿、部分成功回执延续 M0。relink 成功改变根目录后清除下次启动确认，保留历史快照供比较。

## 清单来源

解析结果包含 `project_root / manifest_source / manifest_path / manifest`。source 为 project 或 external，登记指向唯一生效来源，不建立 overlay 合并。

- 项目内清单：`<project_root>/apphost.toml`。
- 外置清单：`<data>/manifests/<installation_id>.toml`，cwd 仍相对 project_root。
- 切换来源先校验目标，再持锁切换登记；原文件保留，不静默删除或覆盖。
- 普通文件校验延续 `validate PATH`；外置校验显式传项目根目录，不能以清单父目录推导。
- 活动实例上的 detach/relink/来源切换经运行协调层先停止；L0 只负责短锁、校验和持久化，不调用 daemon。

登记的 `manifest_source` 缺省为 project；external 只映射到安装 UUID 对应的固定路径，不接受任意外部文件路径。`scan --external` 仅将新推断候选的目标改为外置，不改变已登记来源或已有项目清单。外置候选使用 Draft v2 的 `external: {installation_id, sha256}`，预留安装 ID 并记录已存在文件的摘要；旧 v0/v1 草稿携带外置目标时拒绝。登记失败后保留目标文件与摘要，原草稿只可复用完全一致的声明，不覆盖差异文件。重扫将外置安装视为项目边界；后来出现项目清单时取消默认选择并提示来源冲突。来源索引读取受登记表 1 MiB 限制，不消耗项目发现输入预算；外置声明仍计入扫描的文件读取预算。来源识别失败作为结构化诊断，导入时重新严格核验。

`validate FILE --project-root DIR` 按显式项目根校验。`manifest read/review/save` 共用生效来源及原始文本摘要冲突检测。`manifest review-switch ID project|external` 显示原/目标路径、全文、revision、两个文件摘要与目标启动摘要；目标不存在时复制当前文本，存在时验证并复用，不覆盖。`manifest switch` 提交 revision、base_sha256、target_source、target_sha256（缺省表示目标不存在）和 confirmed_launch_sha256，经 `manifest-switch` RPC 先停止活动实例，再持登记锁重验并发布来源指针及确认。停止或复核失败不切换，不自动重启。来源切换保留两个文件；detach 回执列出项目内和外置的 AppHost 创建文件及原始摘要，均不授予自动删除权限。


## 已登记安装的启动准备

`registry::launch_snapshot(data_dir, installation_id)` 只读取得当前登记的唯一生效清单，验证登记未解除、源码根目录仍为登记的规范化目录、project_id 一致、声明结构有效且确认摘要匹配。缺失登记返回 NOT_FOUND，解除登记返回 DETACHED，源码不可用返回 SOURCE_MISSING，确认缺失或行为字段改变返回 REVIEW_REQUIRED。历史确认快照只用于核验，不能在清单缺失时充当启动声明。

Registry v1 的 installation 带 `revision: u64`，从 1 起步；历史记录缺省为 0，读操作不补写。每次实际改变安装记录时递增，相同内容的幂等写入不递增；其他安装的修改不使当前安装失效。Registry v0 迁移时初始化旧记录的修订号，并补全可确定的历史确认根目录，保留原确认摘要。已有 v1 记录缺少 revision 时，只在该记录实际改变时初始化，不因其他安装的写入而失效。计数溢出明确报 REGISTRY_INVALID，不回绕或重置。该修订号只识别经过登记写入接口的变更，不用于对抗同用户直接篡改或恢复登记文件。

返回的 `LaunchSnapshot` 固定 data-dir、installation、Manifest 和读取时的登记格式版本；字段不允许调用方修改，不反序列化外部提交的快照。对快照的只读访问不授予启动权限。显示名、描述、注释不会单独要求重新确认；v0 保留原摘要算法。外置清单来源切换仍遵循前一节契约。

`runtime::LaunchPlan::prepare(data_dir, installation_id)` 将已确认的声明与当次共享环境组合，预检全部服务后才返回。准备过程不启动进程、不写 Runtime State、不创建配置。供 CLI 展示的 summary 只含安装 ID、项目根目录、声明版本与摘要、服务名、cwd 和解析后的可执行路径，不输出 argv、env 或历史确认快照。

`LaunchPlan::with_current(self, data_dir, action)` 是运行层同步 spawn 的接入点：核对 data-dir，持登记短锁重读当前安装、清单、确认、格式版本和该安装 revision；重新检查 cwd 与可执行路径解析结果。安装已改变返回 STALE_LAUNCH，声明未经确认返回 REVIEW_REQUIRED，解除登记返回 DETACHED；均不调用 action。通过后 action 接收准备好的服务命令，锁保持到同步派发返回；无论成功或失败都会释放。action 不得等待就绪、日志或客户端 IO。只固定声明和当次环境，不签名脚本、可执行文件或依赖内容；文件系统路径核验与进程执行之间仍不是原子事务。

运行层仍负责安装队列、活动实例、端口预检、先持久化启动意图、部分 spawn 失败的资源清理、已启动服务保留与后续就绪。这个接入点不另建运行状态，不把预检通过视作 App 已运行。

`environment check <路径>` 继续只检查声明与环境；`environment check --installation <UUID>` 使用上述登记信任链路，二者互斥。结果表示准备通过，不表示运行、就绪或免除启动前复核。

## 运行环境基线

macOS 未保存 PATH 时，先读取 `/etc/paths`，再按文件名顺序读取 `/etc/paths.d`，每行一个绝对目录；保留路径顺序并去重，再补齐 `/usr/bin:/bin:/usr/sbin:/sbin`，最后追加已存在的 `/opt/homebrew/bin`。只读文件，不执行 path_helper，不扫描运行时；缺少文件可略过，非法内容和读取失败明确报错。最多读取 256 个附加文件、合计 64 KiB，最终 PATH 仍受 256 项 / 16 KiB 限制。共享配置保存在当前 data-dir 的 `environment.json`，格式为 `{schema_version:1,path:字符串,capture?:{shell:绝对路径,captured_at_ms:时间}}`；缺少文件时只读返回默认值，不创建配置。只接受已知版本和字段；损坏配置明确报错，不能静默退回系统 PATH。手动保存会移除旧的采集来源信息。

手动保存的 PATH 必须是非空绝对目录列表，最多 256 项、合计 16 KiB，拒绝空项和控制字符；保留顺序，不展开 `~`、变量或 shell 表达式，不要求目录当前存在。写入先校验，再使用独立短锁和原子发布；不在环境锁内取得登记锁或启动进程。登记与环境写入仅以 700 创建新目录；Unix 已有 data-dir 必须属于当前用户且无组/其他用户权限，否则返回 DATA_DIR_PERMISSIONS 并提示选择私有目录，不修改原目录权限。

启动准备从空环境开始：HOME、USER、LOGNAME 和 TMPDIR 从当前系统用户信息取得，LANG 使用 `en_US.UTF-8`，PATH 使用共享配置，再叠加服务显式声明的 env。服务 PATH 保留 M0 的语义：相对项及空项以已校验的服务 cwd 为基准，转换为绝对目录后同时用于命令解析和子进程环境。不存在隐式的终端环境、`.env` 或 shell 配置继承。

命令仍为参数数组，使用最终 PATH 解析为明确的可执行路径；带路径的命令相对于服务 cwd。准备阶段不执行程序或 `--version`，找不到可执行文件返回 INVALID_COMMAND，并提示通过 `environment set-path` 手动配置或显式 `environment capture --shell /bin/zsh --yes` 采集；不自动执行修复。准备结果不授予启动权限，运行层仍须完成登记确认与 spawn 前复核；配置改变只影响后续准备的启动。

CLI：`environment show` 只读显示 PATH；`environment set-path <PATH>` 显式保存；`environment check <项目或清单>` 用同一环境准备器显示服务 cwd 与可执行路径，不运行服务、不显示 env 值。M0 的静态发现提示保留兼容行为，不能作为 M1 启动预检的替代。

`environment capture --shell <绝对路径> --yes` 是主动执行入口：要求所选 shell 支持 `-il -c` 和 POSIX printf/变量语法，执行交互式登录启动脚本（包括 zsh 的 `.zshrc`，也可能执行主题和补全初始化）后提取 PATH。采集使用同一受控环境，独立进程组、5 秒期限及 stdout/stderr 合计 64 KiB 上限；用本次随机标记分离启动脚本的混杂输出，不输出原始 stdout/stderr。采集结束清理能够证明归属的剩余进程，归属不确定则报错，不重试。取得有效 PATH 后才持环境锁保存；如果采集期间配置改变则返回 CONFLICT，不覆盖较新的配置。超时、超量、非零退出和无效结果均保留原配置。扫描、show、check 和 daemon 启动不调用这个入口。

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

每段有容量上限；创建新段后按最老段清理，原始文件和对应索引一起清理。缓冲、队列及索引都必须有界；超长行和二进制数据不引发无界分配。普通留存默认按安装总计 7 天 / 100 MiB，可通过日志保留设置修改，失败现场另存，不能仅对单个 stdout 文件应用整个安装的预算。

`logs::read_window` 按采集观察时间的闭区间读取完整字节块，单次正文上限 1 MiB、最多 4096 块、索引扫描上限 16 MiB。输出标明前缀被轮转淘汰、未完成的索引尾行、截断、序号缺口和时钟回退；原始数据缺失或索引越界返回错误。单块超过本次字节预算时不切块，返回截断标记。读取期间轮转导致文件消失要报告缺口，不将空结果描述为完整现场。窗口读取本身不证明采集器一直存活，也不证明失败后 10 秒已经结束，这些由失败采集流程记录。

IO 失败向调用方返回错误，不切换到另一种隐藏存储。采集器异常退出（包括无法继续写入而退出）时，实例标记 FAILED，记录日志缺口并保留仍存活的服务；只接受用户全部停止或完整重启，不自动重建管道或采集器。正常 EOF 不等于采集故障；主动停止期间的采集故障仍记录缺口，但不能取消 STOPPING。不能静默丢弃且声称完整日志。服务能否继续存活取决于自身对 SIGPIPE/EPIPE 的处理：Rust 子进程启动会恢复默认 SIGPIPE；shell 或普通 C 程序可能直接结束，Node/Python 等运行时通常自行忽略 SIGPIPE，但仍可能因写入错误退出。管理器不替服务屏蔽信号、不保证服务存活。

## 进程身份与停止

macOS 进程身份使用 PID、微秒级启动时间、有效 UID、进程组和系统启动时间。每个服务由系统 API 启动为独立进程组，不能使用当前管理进程所在组。记录只表示观察到的身份，不授予任意 PID 管理权。

组成员刷新必须有仍匹配的已知成员作为身份依据；leader 退出后，已核验的后代可继续作为依据。若只剩没有核验过的进程，返回 MANAGEMENT_UNCERTAIN，不发信号、不认领、不重新启动。每次 SIGTERM / SIGKILL 前重新核验身份；停止后重新枚举，只有确认没有存活成员才报告完成。退出的组长在整个进程组清空之前不得回收：使用 `observe_exit(&Child)`（macOS waitid + WNOWAIT）观察状态，不能用会回收的 `try_wait()`；清空后才 `Child::wait()`。这保留组长 PID，缩小 PGID 重用窗口；身份核验与发信号仍非原子操作。主动脱离进程组的程序不在支持范围。

停止先发 SIGTERM，等待声明的停止期限，再发一次 SIGKILL 并有界等待退出；不循环强杀。该平台原语只接收已经确认并固定的启动命令，不负责登记信任、队列、daemon 对账或持久化。

## 运行接口约束

### daemon 单例与传输

macOS 入口为 `apphost daemon serve`，以前台进程承载 daemon，便于 CLI 拉起和 LaunchAgent 复用同一个可执行文件。`daemon ping` 仅握手，`daemon check-launch <installation_id>` 仅经 socket 预检已登记安装；两者均不自动拉起 daemon。离线返回 DAEMON_UNAVAILABLE，不创建数据目录或把历史记录当作实时状态。

data-dir 先规范化，复用私有目录规则。`daemon.lock` 使用独占 OS 文件锁，锁持有期覆盖 listener 生命周期；`daemon.sock` 权限为 600。只有拿到锁后才能处理旧 socket，活动 listener、非 socket 文件和符号链接均不覆盖。macOS socket 完整路径必须少于 104 字节，超限报 SOCKET_PATH_TOO_LONG 并提示选择较短的数据目录，不另设隐藏端点。服务端和客户端均通过 getpeereid 校验对端有效 UID 与当前用户相同；握手回传规范化 data-dir，客户端再次核对。

协议版本为 1。请求 request_id 为非空、最多 128 字节的可见 ASCII 字符串，method 最多 64 字节，params 必须为对象；可选 actor 仅接受 human / agent / system，缺省为 human，拒绝其他未知信封字段。actor 只标明自报归属，不改变权限；CLI 从 APPHOST_ACTOR 读取，非法值在修改或拉起 daemon 前返回 INVALID_ACTOR。短 RPC 每连接一条请求和一条响应，然后关闭；后续订阅使用单独连接。请求正文最多 64 KiB、响应正文最多 1 MiB，以换行结束；不完整帧、无效 UTF-8/JSON、大小超限明确失败。响应携带 version 和 request_id，以及 result 或 error 二者之一；无法识别请求时 request_id 为 null。版本不兼容返回 PROTOCOL_VERSION_UNSUPPORTED，不静默降级。

同时最多接纳 16 个 RPC，超额连接立即关闭；客户端收到 DAEMON_UNAVAILABLE，可显式重试。单次请求读取和响应写入各有 2 秒绝对期限，缓慢逐字节发送不能刷新期限。阻塞的文件预检在有限工作线程中执行，不占用接受连接的主循环；无请求时主循环阻塞等待，不轮询。

`hello` 参数为空对象，返回 protocol_version、daemon_id（每次进程启动重新生成）、pid、data_dir 和 capabilities。客户端先完成同版本握手再发送其他 RPC。`check-launch` 仅接受 installation_id，返回与 LaunchPlan::summary 相同的预检结果，不执行服务、不持有可供客户端复用的启动授权。只有 capabilities 声明的已实现方法可以调用；未知方法返回 METHOD_NOT_FOUND。运行操作、事件订阅和历史状态查询按下述契约接入，不能用空状态响应冒充已完成对账。

### 有界事件与日志订阅

`status` 在线快照附带 `{daemon_id, sequence}` 游标，实例写入在持有该实例锁时发布观察；生成快照按安装顺序锁定所选实例后取得游标，避免快照与订阅之间丢失更新。游标仅属于本次 daemon 会话，与操作日志和日志采集序号分别使用。

`subscribe` 使用独立连接，参数为可选 cursor 和 installation_id。无游标时先返回 snapshot；有游标时先验证并返回 subscribed，随后回放严格递增的事件。事件种类包括 instance（运行观察）、wake（自动唤醒暂停）和 failure（现场已更新，携带 failure_id 供重新读取）。环形缓冲最多 256 条、4 MiB；单条超过 256 KiB 时发送 resync_required，要求重新获取快照。过期、跨会话或超前游标返回 EVENT_CURSOR_EXPIRED 并关闭连接，不能静默跳过。无事件时阻塞等待，每 30 秒一条 heartbeat；安装过滤仍推进全局游标。

事件和日志订阅合计最多 8 个，仍受总连接数 16 的上限约束，为短 RPC 保留余量；每条写入期限 2 秒。生产者只发布有界内存观察，不等待客户端 IO。客户端断开仅关闭订阅，不取消运行操作。CLI `watch [--installation-id UUID] [--cursor JSON]` 输出 JSON Lines，不自动拉起 daemon。

`logs --follow` 使用 logs-follow 流式方法，固定 installation_id 和 instance_id，以 after（最后消费的采集序号）续读，limit 为 4096–131072 字节，默认 65536；不与时间窗口混用。每帧返回 window、next_cursor、complete 和 log_incomplete，原始字节仍不改写。轮转淘汰游标前缀时通过 prefix_evicted 明示缺口，后续块不重复；读取到实例输出结束后发送 complete=true 并关闭。只在有订阅时每约 500 ms 有界读取，不增加常驻日志轮询线程，慢读不持有安装锁。

### 生命周期与持久化

显式 `start / stop / restart / stop-all` 在 daemon 不在线时，可以用当前 AppHost 可执行文件拉起 `daemon serve`，在 5 秒内完成同版本握手；启动错误提示用前台 serve 查看原因，不另选 data-dir 或端点。`status` 不拉起 daemon：在线返回 `live: true`，离线只读取历史并明确返回 `live: false`。`logs` 只读且不拉起 daemon。运行方法必须出现在 hello.capabilities 中。

`start / stop / restart` 的回执包含 installation_id、instance_id、operation_id、operation_done 和当前 state。回执表示请求已经接受；调用方通过 `status` 等待同一 operation_id 的 operation_done，读取 operation_error 或 operation_result。客户端断连不取消已接受的操作。重复 start 合并同一 STARTING / RUNNING 实例，不重新执行声明；新实例仍须使用 LaunchPlan 完成当前确认核验。FAILED 且仍有服务、采集器或不确定身份时，start 返回 RESTART_REQUIRED。

运行快照保存在 `<data>/runtime/<installation_id>.json`，schema_version 为 1。记录安装、实例和操作身份、五态状态、各服务的安全日志键、声明端口与有效超时、spawn 意图、进程组身份、存活/就绪观察及退出证据；采集器身份单独记录。未取到退出码时明确记录 exit_status_unavailable。Runtime State 仅由 daemon 写入，采集器不参与。原子发布需要同步文件和父目录，启动意图及 spawn_attempted 必须先于相应系统 spawn；拿到 OS 身份后再发布身份检查点。发布中断留下的专用前缀临时文件不参与对账，取得单例锁后只清理可识别的未发布普通文件；损坏的已发布快照明确报错。runtime 仅识别规范 UUID.json，instances 仅识别规范 UUID 目录，日志根仅识别规范十进制段号；以点开头或名称不符合约定的目录项跳过，每次扫描合并一条结构化警告，不删除、不写入 stderr。离线 status 在顶层 warnings 返回本次扫描提示；在线 status 顶层返回本次 daemon 对账提示，实例 warnings 返回其启动清理提示；日志读取在 window.warnings 返回本次扫描提示。提示采用 code / level / title / evidence / actions / clears_when，仅表示对应扫描时的观察，下次同类扫描覆盖，不改变运行状态。每次最多扫描 4096 个目录项；名称合法但内容、类型或身份不合法的权威记录仍报错，符号链接不能成为记录或日志段。

每个安装最多一个工作线程，最多同时管理 32 个活动实例、每实例 32 个服务、1024 个安装快照。不同安装的同步启动派发使用 daemon 内互斥锁串行执行；启动路径获取登记文件锁最多等待 2 秒，获得锁后重新复核快照，超时返回 BUSY，不自动重新执行操作。其他登记命令保持原有非阻塞语义。日志归档清理、采集器就绪及登记文件锁等待均在派发锁外。取得登记锁并完成复核后才获取派发锁，仅覆盖同步 spawn 及其身份检查点；排队时不持有安装快照锁或全局安装表锁。派发锁不承载可变数据，单个安装 panic 后允许恢复这把锁，不据此恢复损坏的安装记录；就绪期限从派发完成起计。不同安装独立等待就绪及停止，RPC 不等待这些长操作；相同安装的 Stop 与同步 spawn 派发在短临界区内排序。先停止再重启的后半段重新准备当前声明并预检端口，不复用旧命令。同步 spawn 返回错误后停止派发尚未启动的服务，关闭未使用管道；已经启动的服务保留为失败实例的诊断对象。

就绪检查仅连接声明的 loopback 端口，并通过 macOS libproc 的监听 socket 信息及当前进程身份，证明监听者属于该服务进程组。其他进程抢先监听不能满足就绪。无端口服务连续存活 2 秒后只表示进程就绪；声明的启动期限耗尽返回 READINESS_TIMEOUT。启动时每约 100 ms 检查一次；进入 RUNNING 后每约 1 秒用 libproc 被动核验监听归属，不再向服务建立探测连接。进程组消失或观察到服务退出仍判失败。运行中端口丢失时保留 RUNNING、ready=false 作为就绪告警；从首次观察丢失起按单调时间给予 10 秒宽限，恢复后清除告警和计时，连续满 10 秒后记录 PORT_NOT_READY。已经失败的实例不因端口恢复而自动恢复 RUNNING。停止意图仍以约 100 ms 的轻量检查响应，未到观察时间不刷新进程组、不枚举 FD、不序列化记录。端口预检分别试绑 IPv4/IPv6 的通配与 loopback 地址，共四处，以覆盖 macOS 允许可复用通配地址与具体地址并存的行为；保留 SO_REUSEADDR，避免把已经关闭服务的 TIME_WAIT 当作活动监听。任一地址占用即拒绝；预检不保留端口，实际启动仍须核验监听归属。占用返回 PORT_IN_USE，其他试绑错误返回 PORT_CHECK_FAILED。冲突结果包含可取得的占用 PID/名称；占用者查询最多扫描 65536 个 PID、约 250 ms，IPv4/IPv6 合并一次 FD 枚举，找到一个代表性监听者即结束。结果 scope=first_match，owners 至多一项，found 表示已取得占用证据；incomplete 仅表示未找到且本次查询受限，不承诺列全所有监听者。limitations 分别记录 permission_denied、unavailable、timed_out、pid_limit 与 pid_list_failed；界面优先展示已找到的进程，找不到且受限时才提示信息不可得，该结果不授予停止外部进程的权限。

无声明端口的服务在首次就绪时采集一次已证明归属的 loopback TCP 监听端口；就绪超时也采集一次并附入失败详情。`observed_listeners` 含 ports、incomplete、scope=owned_loopback_tcp 与 observed_at_ms，仅作诊断建议，不生成 open 或 URL。单服务最多检查 128 个已核验进程、每进程至多 1024 个 FD、结果至多 128 个端口，跨进程枚举约 250 ms 后停止；权限、身份变化、容量或期限受限均标记 incomplete，不因此让 App 失败。运行时短暂失去端口产生结构化 PORT_READINESS_LOST 警告，恢复后清除，并分别记录 readiness-lost / readiness-restored 操作事实。

停止各服务先发一次 SIGTERM，等待各自声明期限，再发一次 SIGKILL 并有界核验。只有进程组清空后才回收 Child；新启动但瞬时退出的组长可由 `capture_spawned(&Child)` 在尚未回收时记录身份，不能借此认领未知后代。采集器在服务停止后有 2 秒处理 EOF，仍未完成时显式结束并标记日志不完整。停止期间采集错误不能取消 STOPPING。所有已知组清空且没有未解决的 spawn 身份缺口时，才进入 STOPPED；停止失败不执行重启或后续登记变更。

恢复时先拿单例锁，核验全部已有身份，再接受新启动。系统启动时间已改变的旧组视为已消失；同次启动中非空组必须有匹配的已知活成员。磁盘上的 RUNNING 不能直接恢复为 RUNNING，需要重新验证就绪；仍未就绪的进程进入有期限检查。缺失身份的 spawn 意图、身份不一致或不能完成的核验保留为 FAILED + management_uncertain，不认领、不发信号、不重复启动。没有存活进程的 FAILED 实例仍保留 FAILED 与失败原因，直到显式停止或启动改变状态，不因 daemon 或系统重启消失。已接受的停止、完整重启及停止后的登记变更，其意图随运行快照保存；恢复后仍先停止旧组，成功后才执行后半段。

Stop All 先在版本化 wake.json 中一并持久化 automatic_wake_paused 和递增的取消代数，安装操作记录接受时的代数。恢复时旧代数的活动实例仍须停止，旧代数的重启后半段不得继续；这覆盖全局意图已发布而安装记录尚未更新的崩溃窗口。新的显式操作使用当前代数，不因自动唤醒仍暂停而被拒绝。该代数只用于 Stop All 取消，不替代每安装独立的登记 revision。随后取消尚未派发的启动和重启后半段，向各安装独立提交停止；返回逐安装操作回执或错误，最终结果从对应快照读取。显式 start 只授权本次运行，不解除自动唤醒暂停。

wake.json 缺失且存在运行记录时，以最大 operation_epoch 加一恢复取消代数并保持自动唤醒暂停，取消未完成的重启后半段，继续已持久化的停止意图，保留正常运行的服务。先逐条保存统一代数和取消后的意图，最后发布 wake.json；中途再次崩溃时仍按缺失文件恢复，不将此次归一化解释成 Stop All。没有运行记录时初始化为未暂停、代数 0。代数溢出、已有文件损坏或版本不支持仍明确报错，不猜测内容。

显式 `resume-wake` 不接受参数；与 Stop All 共用短写入门，持久化 automatic_wake_paused=false，保留当前取消代数，不重放已取消的启动、不启动任何 App。daemon 离线时允许拉起以完成这次显式操作。显式 start 仍不等同于恢复自动唤醒。

活动或有运行历史的安装，CLI 的 detach / relink 经 daemon 协调；离线时先拉起并对账。协调请求成功停止后才调用 L0 登记写入，长操作的登记结果位于 operation_result。无 daemon、无运行记录的 M0 登记操作仍使用原 L0 入口和回执。批量 detach 在提交任何停止前核验所有安装 ID，并去重，未知 ID 不产生部分停止或登记写入。relink 的 RPC 路径必须是绝对路径，由 CLI 在发送前规范化；停止旧实例不依赖当前源码或清单仍可读取。

### 实例日志读取与普通归档

每实例的独立采集器继承父进程分配的有限描述符列表，一次读取全部服务的 stdout/stderr；不提供由管理 RPC 注入描述符或任意日志路径的入口。服务名只显示，文件使用 s0、s1 等安全键。日志位于 `<data>/instances/<instance_id>/logs`，每段 1 MiB、最多 8 段，包含原始数据和观察索引。采集器正常 EOF 写完成标记；服务主动关闭输出流而继续运行不算采集故障。

`logs` 窗口输入为 installation_id、instance_id、from_ms、to_ms、limit；时间为观察时间的闭区间，limit 为原始字节预算，1–131072，默认 65536。响应包含安全键与服务名映射、原始字节块及窗口缺口标记、实例 log_incomplete。JSON 字节数组会膨胀，仍受统一 1 MiB 响应上限。日志读取在独立 RPC 中进行，不占用生命周期工作线程；后续跟随读取不能阻塞停止。

新实例替换已完成实例前，保存旧实例的只读 record.json，历史日志仍按 installation_id / instance_id 校验归属读取。启动和完整重启时清理过期或超量的普通归档：安装预算默认 100 MiB，给当前实例预留 9 MiB，旧归档按创建时间淘汰并应用默认 7 天年龄上限；清理开始时读取当前日志保留设置；不删除活动或管理权不确定的实例。扫描上限 4096 个实例目录、15000 个归档条目；异常目录、链接或超限明确失败，不猜测归属。普通日志归档不承载 D9 的独立失败现场。

### 独立失败现场

`failure` 接受 installation_id 和可选 failure_id；不指定 failure_id 时读取最近一次失败。CLI `apphost failure <installation_id> [--failure-id <uuid>] [--json]` 直接读取持久化现场，不启动 daemon 或 App。响应包含 failure、history、details_expired 和连续同因观察次数。没有现场目录时返回 NOT_FOUND；已知现场超过保留期限时返回 failure=null、details_expired=true，保留最近失败的摘要。损坏的权威记录明确失败，不当成空历史。

现场位于 `<data>/failures/<installation_id>/<failure_id>.json`，schema_version 为 1。记录包含安装、实例（预检失败可以为空）、服务、phase（preflight/startup/runtime）、观察时间、底层 cause_code、分类 evidence、脱敏详情、日志快照、stderr_tail、collecting 与 gaps。evidence 带 code、rule_id、inferred、匹配片段和所检查 stderr 文本中的行号；具体依赖、端口或版本错误优先于泛化退出，运行中的普通退出归 UNKNOWN。运行状态中的底层失败原因继续保留，分类结果不改写生命周期。

daemon 使用一条独立的有界采集队列（64 项），最多保留 32 个待补尾窗口；空闲时阻塞等待。生命周期不等待失败后的 10 秒，容量或 IO 错误通过 status.warnings 报告 FAILURE_CAPTURE_INCOMPLETE。首先保存可读现场，再异步补齐观察区间 `[失败时间-30秒, 失败时间+10秒]`。初次与最终日志分别标记 initial/tail，可能重叠，不伪装成去重后的连续日志。每次日志快照读取至多 64 KiB 原始字节，stderr 独立保留至多 32 KiB 的最新字节用于分类，以免 stdout 洪泛挤掉报错；展示的 stderr 尾部至多 8192 字符。跨采集块的文本先按服务和流合并，再匹配和脱敏。

环境确认快照中的值、凭据特征行和本机用户路径默认脱敏；任意程序输出无法保证完全识别，返回 redaction 提示供分享前复核。JSON 转义和脱敏可能扩大正文，最终现场受 900 KiB 上限约束，必要时截断输出并记录 evidence_byte_limit，为 1 MiB RPC 信封预留空间。读取、轮转、采集器退出和 daemon 恢复的缺口均显式标记。daemon 离线且补尾期限已过时，读取将未完成的 collecting 显示为 false，并添加 tail_capture_not_completed，不永远显示采集中。

写入时按每安装最多 20 条、30 天清理，读取不创建目录、不清理文件。最后失败引用与最近成功时间保存在独立 history.json 中，普通日志被清理不影响已保存的现场；迟到的补尾不能重新插入已被较新失败淘汰的记录。连续同因次数从上次成功之后的留存记录计算；达到留存边界时 count_limited_by_retention=true，调用方只能将该值显示为下界，不能声称完整历史次数。

同一 history.json 保存 last_started_at_ms、last_finished_at_ms、last_result、last_run 和 last_success_context；`history <installation_id>` 与同名 RPC 只读返回这些事实，无记录时为空，不创建目录。启动准备固定分支、声明摘要、生效来源及共享 PATH 的摘要；只有真正进入 RUNNING 才更新成功基线。缺失的历史确认发起者保持 null，不推断为本次启动者。运行快照先持久化，再更新历史，失败以 HISTORY_WRITE_FAILED 提示，不阻断停止；daemon 对账可从快照补齐。旧实例的迟到观察不能替换较新的成功基线。failure 返回 changes_since_success，比较当前可读取的分支、声明和环境摘要；缺少基线或当前源码不可读时明确 unavailable，不用旧数据冒充当前事实。不存储依赖指纹或环境原值。

运行状态仅 STOPPED / STARTING / RUNNING / STOPPING / FAILED；就绪、退出原因和 management_uncertain 为附加信息。操作 ID 用来关联长请求，不另建持久化任务状态机。

请求采用 JSON Lines：`{version, request_id, method, params, actor?}`；响应回显 request_id，成功返回 result，错误返回沿用 M0 的 `{error:{code,message,details}}`。长操作返回 operation_id 后通过状态查询/事件获得结果；断连不取消已接受的操作。事件游标与快照绑定，过期游标要求重新读取快照。

| 方法 | 输入 | 行为 |
|---|---|---|
| start | installation_id | RUNNING 幂等；STARTING 合并；FAILED 仍有存活服务时要求 restart；启动前核验当前声明 |
| stop | installation_id | 可取消 STARTING；退出已证明归属的全部进程后才报告 STOPPED |
| restart | installation_id | 先停止旧实例，成功后重新校验并分配新 instance_id |
| status | installation_id 可选 | 返回当前状态、活动服务与提示；daemon 离线不将历史视为实时 |
| history | installation_id | 只读长期运行事实和最近成功基线；不自动拉起 daemon |
| logs | installation_id、instance_id、from_ms、to_ms、limit | 有界观察窗口；返回服务映射和缺口标记，慢读不阻塞 stop |
| url | installation_id | 只读解析稳定入口，不拉起 daemon 或 App |
| failure | installation_id、failure_id 可选、export 可选 | 返回结构化失败和核心建议；export=true 返回诊断预览 |
| stop-all | 无 | 暂停自动唤醒、取消待启动、停止所有活动服务，返回逐项结果 |
| resume-wake | 无 | 显式解除自动唤醒暂停，不启动 App、不改变取消代数 |
| journal | cursor、limit | 有界读取操作日志 |

网关默认端口为 18480，保存于当前 data-dir 的 gateway.json（schema_version=1、port）。daemon 首次成功启用网关时持久化默认值；`gateway show` 与 `url <installation_id>` 均只读，不创建文件或拉起 daemon。`gateway set-port <port>` 显式修改设置，在线时先绑定新端口再保存并切换监听；失败保留旧端口，不静默换端口。网关绑定或配置失败作为独立的 gateway 诊断出现在运行快照中，不阻断管理 socket、Stop All 和日志查询。Windows Job 生命周期仍按阶段稿的验证要求确定；未支持的平台能力明确返回 UNSUPPORTED_PLATFORM。

新安装在首次登记时保存 slug：初始标签转换为至多 30 字节 ASCII 小写名称，加完整 installation_id；不能形成名称时用 app。改名、relink、重新登记不改变 slug；历史未保存 slug 的安装固定使用 app 加完整 installation_id，读取不迁移登记表。slug 后缀必须与安装身份相符，防止另一个安装复用历史地址。入口由已确认的 open_target 得出，无端口返回 url=null，多端口缺少 open 返回 OPEN_TARGET_REQUIRED。

HTTP 网关独立监听 IPv4/IPv6 loopback，仅接受 HTTP/1 的 origin-form 路径、一个精确的已登记 Host（包括配置端口），拒绝 CONNECT 和客户端指定 upstream。每个请求重新检查登记与确认；只向对应 RUNNING 实例中已证明归属的声明端口转发。HTTP、上传和响应采用流式背压，WebSocket 透传双向字节；保留 Host / Origin，移除客户端 Forwarded / X-Forwarded-* 及逐跳头，再生成 X-Forwarded-Host / Proto。不开通 CORS 管理接口，不合成 /api 路由。每个网关最多 64 条连接（包含升级后连接）、128 个头部和 16 KiB 头部缓冲；请求头及 upstream 连接期限各为 5 秒，upstream 响应头期限为 120 秒。阻塞的登记、身份和生命周期工作通过有界连接对应的后台任务执行，不占住 HTTP 事件循环。

保留 `/__apphost/status/<operation_id>` 为只读状态轮询路径，只接受 GET，不执行生命周期操作；静态页不缓存、禁止嵌入 iframe，脚本只允许本页 nonce，失败时提示从 AppHost 显式处理，不提供 HTTP 重试按钮。

`gateway::is_wake_navigation(method, raw_headers)` 是无副作用的导航判定，输入保留重复头部，不读取登记表或启动进程。必须满足 GET、Sec-Fetch-Dest=document、Sec-Fetch-Mode=navigate、Sec-Fetch-Site=none；Sec-Fetch-User 可缺失，出现时只能为 ?1。Sec-Purpose 或 Purpose 明确包含 prefetch / prerender 时拒绝。相关头部重复、控制字符、超长值或必需值不符均拒绝；名称不区分大小写，元数据 token 精确匹配。不通过 UA 白名单绕过缺失条件。

导航判定通过仍需核对 loopback 连接、已登记 Host、安装信任与自动唤醒暂停状态。静态资源和状态轮询不能借此启动。该规则接受具有相同元数据的恢复标签页或直接导航重定向可能触发唤醒；G2 覆盖这些情形，并验证 Safari 实际书签导航，不能仅凭地址栏测试判定通过。

符合条件的浏览器唤醒复用安装队列，操作事实为 browser-wake / system。同一安装已在启动或运行时合并到当前 operation_id；存在失败原因时只返回该次失败，必须由 AppHost／CLI 显式启动或完整重启才能创建新尝试。停止和 resume-wake 不清除失败阻止自动重试的语义。确认和来源检查未通过不接受启动；已通过信任检查后的环境与端口预检失败保存为没有派发进程的 FAILED 快照，保留 operation_id 和原因，daemon 重启后继续生效。不增加运行状态或另一份唤醒重试状态。

唤醒准备不持全局安装锁；接受前在 Stop All 的同一短临界区复核暂停状态、取消代数和已有实例，准备期间发生 Stop All 后即使又 resume-wake，旧请求仍返回 START_CANCELLED。静态页轮询只读匹配的 installation_id / operation_id，响应仅包含状态、是否就绪、是否完成与原因码；操作已被显式操作替换则返回过期，不跟随新操作、不重新启动，也不暴露命令、路径、日志或环境。

### 操作日志

操作事实保存在 `<data>/journal/<十进制段号>.jsonl`，独立于 stdout/stderr、失败现场和未来的实时事件订阅。每行版本为 1，字段为 event_id、at_ms（Unix 毫秒）、action、actor、installation_id、instance_id、operation_id、outcome、code；不适用的身份为 null。只记录身份、动作和结果码，不复制命令、环境值、路径、标签正文或原始输出。发起者随长操作保存，daemon 重启不将原来的 agent 操作改成 human。

outcome 为 accepted / succeeded / failed / canceled / observed。运行操作发布意图时记 accepted，真正完成后记最终结果；重复 start 合并同一操作，不重复生成接受记录。完整 restart 保持同一 operation_id，接受记录关联旧实例，成功记录关联新实例。新停止请求取消尚未完成的启动时，先记旧操作 canceled，再记停止 accepted。已经成功进入 RUNNING 后发生故障，新增 system 的 runtime-failure / observed 事实，不改写先前启动成功的结论。Stop All 的全局记录表示逐项停止是否已接受，有未完成项时为 accepted，最终结果通过逐安装操作关联；不创建额外的全局任务状态机。恢复补发停止时，保留原操作取消事实，新停止归属 system。

L0 写操作由 CLI 应用层读取核心回执后记结果；部分批量成功按 completed 中的安装逐条记录，再记本次失败，不把失败项记为成功。daemon 已返回 operation_id 或 journal_event_id 的回执不在 CLI 重复记录。因显式运行命令拉起 daemon 时，另外记录 daemon-start 的就绪结果。只读命令不新增操作日志、不拉起 daemon。

每段至多 1 MiB、稳定状态最多 8 段，写入时清理整段过期数据及超额最老段；保留最近 30 天且受 8 MiB 总容量约束，容量先达到时提前淘汰，不能声称完整 30 天。轮转发布与清理之间崩溃可临时有第 9 段，下次写入先恢复容量。跨进程读写使用独立文件锁，至多等待 200 ms；目录枚举最多 256 项，每行至多 4096 字节。不认识的名称跳过；合法段名的损坏内容、符号链接或非普通文件明确报错，未完成尾行不静默截断。

CLI `journal [--cursor <event_id>] [--limit <1..200>] [--json]` 和同名 RPC 返回 `{events, next_cursor, has_more}`；limit 默认 100，按追加顺序读取，cursor 指最后读到的事件 ID。游标被轮转或时间留存淘汰时返回 JOURNAL_CURSOR_EXPIRED，调用方重新读取；无日志时返回空数组，不创建目录。读取不做清理，也不把日志游标用作实时订阅游标。

运行快照先原子保存业务结果与待补事实，再尝试追加日志；每安装待补最多 32 条，在后续状态保存或 daemon 对账时重试，以稳定 event_id 去重。同一 event_id 的事实内容冲突时返回 JOURNAL_CONFLICT，不覆盖旧记录。写入失败通过 warnings 的 JOURNAL_WRITE_FAILED 标明，补齐后清除；超过待补上限另保留 JOURNAL_INCOMPLETE，说明无法补回的历史缺口。日志故障不能阻止已接受的停止，也不能把业务成功改为失败。离线 L0 写入不另建重试队列，日志失败在当次回执中说明缺口。保持 M0 对象或数组回执形状；错误的警告附在 error.details.warnings，成功命令不向 stderr 输出文本警告。


## 桌面端交互适配（设计契约）

界面展示模型由登记信息与运行观察按 installation_id 关联生成，不在 ui.json 维护一份运行状态。没有运行记录的已登记安装可显示为未启动；live=false 或连接中断的记录明确标记为历史观察。待确认、源码缺失和目录扫描警告作为提示，不扩充五态枚举。启动和停止按钮使用 daemon 操作回执，跟踪同一 operation_id 的完成和错误；迟到的旧回执不得覆盖新的用户意图，Stop All 后不得由前端计时器重新启动应用。

列表和详情消费 warnings、失败原因、服务 ready/alive 及操作结果。结构化警告不等于失败，不根据 stderr 是否非空推导运行状态；CLI 的成功 JSON 查询不向 stderr 输出警告。失败详情的端口信息以 ownership.found 决定是否展示监听者，以 limitations 解释查询受限；scope=first_match 不代表唯一占用者。

桌面入口只承接用户的显式操作，复用核心的 scan/review/import、LaunchPlan 和运行管理契约，不在界面重新实现源码探测、声明确认或进程管理。真正的系统目录选择、核心声明校验、浏览器稳定入口、失败现场与操作记录接入分别沿对应设计实现；交互原型不替代这些能力的验证。正式桌面端采用 Tauri 2 + Vue 3 + TypeScript + Vite，沿用已确认原型的视觉与交互。Vue 组件只负责展示与发出动作，Rust 桥接层调用核心；不引入额外的全局运行状态管理框架。

桌面工程放在 `desktop/`，与根目录 CLI 共用核心库和数据目录解析规则：显式目录优先，其次 APPHOST_DATA_DIR，再使用现有平台默认路径。WebView 不接受任意数据目录、任意文件读取或任意 daemon 方法名；系统操作由具名 Rust command 承接。构建锁定 npm 与 Cargo 依赖；开发服务只绑定回环地址，端口固定且被占用时报错。

桌面包通过 Tauri externalBin 携带同次构建的独立 `apphost`，运行时只从桌面可执行文件相邻位置解析，不接受 WebView 路径或 PATH 搜索结果。`desktop_connect()` 是显式的后台连接/拉起操作，使用既有单例握手和启动审计；只读 snapshot/watch 不拉起 daemon。关闭 UI 只释放观察连接，不停止 daemon、采集器或 App。Windows 的实际运行、系统浏览器打开及系统终端打开仍返回明确的平台不支持结果，沿 G3 决策实施。

`desktop_action(action, installation_id?, failure_id?)` 只接受 start / stop / restart / stop-all / resume-wake / stop-owner-and-start；除 stop-all 外，确认后台未运行时显式操作可通过打包 CLI 拉起 daemon，启动审计沿用核心；连接异常不能按未运行处理。stop-all 只发送给已运行的 daemon，不自动拉起；界面还需有实时的可停止实例或正在提交的启动操作才启用按钮。stop-owner-and-start 以 installation_id 与 failure_id 绑定最近一次失败，Rust 从核心建议提取占用实例，daemon 再核验身份与监听归属；不信任 WebView 提交的占用者身份。返回原操作回执，界面继续跟踪对应 operation_id 的快照，不自动重发超时的写请求。`desktop_watch(channel)` 每个桌面视图使用一个只读订阅，Rust 只向 WebView 推送 connected / changed 通知，收到后合并读取快照；初始订阅自带快照游标，过期或断连要求重新取得快照。重载或显式取消通过 `desktop_unwatch()` 释放旧订阅，空闲读每 250 ms 检查取消，原有帧期限不延长。

快照补充 operation_id、operation_done、脱离原始 details 的 operation_error.code/message、是否有浏览器入口及自动唤醒暂停标志。父目录只提供默认展示分组和名称，不创建后台分组。`desktop_open(installation_id, instance_id, operation_id)` 重新核验实时的同一操作/实例已 RUNNING 且服务存活就绪，随后打开核心生成的稳定 URL，不接受任意 URL。自动打开只绑定本次显式启动意图，消费一次后删除；停止、Stop All、断连或实例变更取消该意图，迟到回执不得恢复它。

`desktop_preferences()` 只读 `ui.json`；外观结构包含 `{schema_version:1, theme:system|light|dark, groups, collapsed, show_recent, last_used, search}`；旧文件缺少新增字段时使用默认值，主题缺省跟随系统。`desktop_theme(theme)` 原子保存主题，损坏或不支持的文件不覆盖；不保存运行状态。`desktop_ui_edit(change)` 以同一文件锁更新单项外观设置，保留其他设置：search、recent、create-group、rename-group、dissolve-group、move、collapse、touch、prune。桌面端顺序提交外观写入，最近使用写入不丢弃同期设置修改；跨进程文件锁最多等待 1 秒，只对锁竞争等待，超时明确返回 BUSY。桌面观察到解除登记或打开旧库时，prune 在锁内按核心当前登记清理已解除安装的分组成员和最近使用引用；保留分组本身、折叠与其他设置，不修改项目。自定义分组最多 128 个、名称最多 64 字符、总成员最多 1024 个且单一归属；折叠键最多 256 个，最近使用时间最多保留 1024 个安装。分组成员按登记顺序展示，移动只改变归属，解散回到默认父目录。最近使用组可关闭，登记数 ≥ 12 时最多显示 6 个图标，该区域以外不按时间重排。搜索设置为回车默认 primary/details、路径匹配开关、建议操作开关；它们只影响界面，显式 `>` 始终查询操作。搜索解析支持中英文状态别名、`状态:` / `组:`、名称及分组匹配；建议列表按前缀、子串、子序列匹配排序，画布保留登记顺序。聚焦展示语法提示，↑↓ 选择结果，回车执行所选行，Shift+回车仅切换应用行的 primary/details 备选操作。搜索期间隐藏没有匹配成员的自定义分组，清除搜索后恢复。颜色和尺寸规则沿用原型：停止为中性底、存活进程填充项目身份色，故障色仅用于角标和提示；磁贴随画布高度计算 1–3 行、窄窗 96–120px / 常规窗 104–148px。

`desktop_snapshot()` 是只读观察接口，不创建目录或启动 daemon，返回 `{apps, live, automatic_wake_paused, read_at_ms, runtime_error}`。`apps` 按登记顺序排列，每项只包含 installation_id、project_id、label、path、group、group_label、registration_state、requires_confirmation、has_entry 与可选 runtime；runtime 仅包含实例 ID、安装 ID、operation_id/operation_done/operation_error、五态、失败代码、记录更新时间、created_at_ms、可选 started_at_ms、management_uncertain 及服务 name/port/alive/ready/startup_timeout_ms。started_at_ms 与派发完成后的就绪计时起点一致，老记录缺失时不虚构期限；准备阶段显示准备启动。该列表接口不向 WebView 传递 confirmed_services、confirmed_manifest、env 或 launch_context。运行查询错误保留为 runtime_error，不把未知状态当作 STOPPED；已成功取得的历史记录保留其五态和更新时间。整个读取失败时前端保留上次成功结果、撤销 live 标识并显示读取错误。没有事件订阅的快照必须显示观察时间，不能宣称持续实时；显式刷新期间合并重复读取请求。

`desktop_detach(installation_id)` 只解除登记，不删除项目；界面明确确认后提交。`desktop_relink(installation_id)` 通过原生目录选择器选取移动后的目录，核心核对项目身份、旧目录不存在及路径冲突；取消选择没有副作用。有实时 daemon 或持久化运行记录时走 stop-and-write 协调，必要时通过随包 CLI 拉起后台；纯登记离线修改不拉起后台。回执中的 operation_id 可通过只读 `desktop_operation` 核对完成与错误；未知结果不重发写操作。重新关联后保持停止，并复核新路径的启动声明。

`desktop_failure(installation_id, failure_id?)` 可读取最近一次或指定历史现场；结果 `records` 返回留存范围内的失败引用（ID、时间、类别），不复制每条完整日志。历史现场的启动/停止占用者按钮不可用。`desktop_condition_watch(installation_id, failure_id, token, channel)` 只读订阅最近失败的端口条件，最多 60 秒；关闭或切换现场用 token 取消，下一次观察关闭连接；桌面允许至多 32 个独立有界订阅，失败气泡与详情不能互相覆盖取消令牌。端口释放仅提示重试，不自动启动。`desktop_journal(cursor?)` 每页读取 50 条核心操作记录，不启动后台；记录按核心追加顺序展示。

`desktop_icon(installation_id)` 只通过核心固定候选发现和摘要校验读取图标，最多 256 KiB；WebView 将返回的 MIME/字节渲染为 data URL，不接收任意本地文件路径，读取失败使用首字。多服务环按各服务 alive/ready 表示就绪程度，历史观察不显示实时绿色。

后台与网关设置使用 `desktop_login_read/save`、`desktop_gateway_read/save`。登录开关需用户明确选择，不预填待提交选择；读取返回 `choice_made`，未选择时桌面首次显示开启/关闭提示。成功保存后，在同一登录配置锁内写 `login-choice.json`（仅 `{schema_version:1}`，不复制 OS 启用状态）；读取不建目录、不写标记，旧有受管 LaunchAgent 视为已选择。登记的可执行文件为打包 CLI，仅下次登录启动后台，不启动应用。网关只按用户指定端口保存，运行中绑定冲突保持旧配置，不自动换端口；离线保存不拉起后台。

`desktop_candidate_external(token,index,external)` 只调整新推断候选的存储位置并轮换草稿 token，不写文件、不覆盖已有来源；最终导入才写入项目或 AppHost 管理目录。`desktop_manifest_switch_review(installation_id,target_source)` 返回当前/目标文本、路径、摘要、revision 与完整目标声明；`desktop_manifest_switch(installation_id,request)` 在显式确认后复核基线，必要时停止实例再切换，原文件保留。声明编辑展示有界差异预览及完整文本、命令参数数组、工作目录和端口摘要；失败编辑入口按服务及相关字段定位，不把日志行号误当作 TOML 行号。

顶栏活动区按 installation_id 与 operation_id 跟踪回执；单应用成功结果在磁贴旁短暂显示，失败气泡保持到显式关闭或条件解除，读取本次失败的核心 advice.title/impact/主要 actions/连续次数；同因重复显示“与上次原因相同”。端口条件解除后改为可以重试，不自动启动；状态先于现场落盘时，最多读取 5 次，间隔合计 2 秒；只接受本次操作时间之后、匹配实例的现场。回执错误或捕获异常的 code/message 立即传入气泡：在没有匹配现场时显示实际原因、操作完成尚未确认的影响及一个主要操作，不用无关历史覆盖，也不提供“详情”按钮；读到匹配现场后才使用核心建议及详情入口。前置与控制错误不虚构失败记录或连续次数；新观察证明确认/路径/后台连接条件恢复或本次之后运行已恢复正常时收起提示，历史成功不能清除本次错误。关闭、重试和新结果取消旧读取与条件订阅的影响。完整记录通过操作记录页读取。Stop All 聚合逐项结果，区分已停止、失败、取消启动及身份不确定的遗留进程。超过 5 秒的启动显示已等待时间及各服务有效期限；不增加前端运行状态机。分组启动尚未派发的成员被 Stop All 取消；成员有更新的显式操作时跳过旧批次意图。

桌面反馈（冻结稿 5.9、10.4）需要接口提供以下数据，前端只做比较和展示，不自行推断或累计：

- 失败记录：code、phase、service、证据（含 rule_id 与是否推断）、输出末尾及匹配行位置；PORT_IN_USE 的占用者归属（AppHost 安装的 installation_id，或外部进程的名称与 PID）；READINESS_TIMEOUT 的已证明归属实际监听端口。
- 连续同因次数：自上次成功以来，同一 code 与 service 的连续失败次数，由 daemon 从失败记录计算；成功后清零。
- 条件解除：端口占用类提示按 clears_when 推送"占用已解除"的观察（受管占用者停止、外部占用者消失）。只在桌面端订阅且提示存活期间检查，频率和时长有上限；解除后只改为提供重试，不自动启动。 `condition-watch` RPC 指定 installation_id、failure_id、duration_secs（1–60，默认 60），共用最多 8 个观察订阅名额，每秒最多检查一次。返回 kind=condition，data.state 为 blocked / cleared / unknown / expired，complete 表示流结束。故障被替换、证据过期或后续已成功时返回 CONDITION_EXPIRED；没有确定声明端口时返回 CONDITION_UNAVAILABLE。只检查已记录的端口，不遍历端口或结束进程。`failure --watch` 使用同一订阅且不自动启动 daemon；断开、解除或期限到达便释放观察资源。
- 自上次成功以来的变化：成功时记录分支、声明摘要及其确认来源（发起者）和运行环境基线版本（冻结稿 9.5），失败详情与当前值比较；没有记录的项不显示，不猜测。
- 启动进度：STARTING 期间提供 started_at 与有效 startup_timeout_ms，用于显示已等待时间和期限。
- "停止它并启动"作为一个复合操作（D10），回执区分停止占用者与启动两个阶段。

`stop-owner-and-start` 接收 `installation_id`（目标）、`owner_installation_id`、`owner_instance_id`。只接受当前进程身份能够证明拥有目标声明端口的受管实例；占用提示中的旧 instance_id 不授予停止新实例的权限。目标先保存复合操作意图及预分配的占用者 stop operation_id，再提交停止；重复的同目标、同占用实例请求合并，目标已有其他操作时返回 BUSY。回执 `operation_phase` 为 stopping_owner / starting_target / complete，后续快照中的 pending_owner.stopped 表示停止阶段是否完成。沿用五态，不增加独立工作流状态机。

占用者成功 STOPPED 后重新准备目标、检查每安装 revision 和全部声明端口，再由现有启动快照复核后派发。等待期限为占用者各服务停止期限加每服务 2 秒强制退出核验及 10 秒采集/协调余量；本次等待使用单调时间，恢复不延长已保存期限。OWNER_CHANGED、OWNER_NOT_PROVEN、OWNER_STOP_FAILED、OWNER_STOP_TIMEOUT、STALE_LAUNCH 均结束本次操作，不自动循环。两个安装以短的全局入队门协调，一次只锁一个安装记录，等待时不持有全局门或占用者记录锁；跨安装操作不会互相持锁等待。

daemon 恢复只续接尚未派发采集器和服务的已保存目标意图，利用预分配的 stop operation_id 识别停止是否已接受；已有派发意图仍按原有身份核验规则处理，不重复启动。Stop All、显式停止目标或缺失 wake.json 的保守恢复取消后半段；已经接受的占用者停止继续完成。

- 批量结果：stop-all 逐项返回 stopped、failed（原因）、canceled（取消的待启动）和 orphan（身份无法确认、未结束的进程及证据）。分组只是 ui.json 的外观数据，daemon 不感知分组：分组启停由桌面端对各安装分别发起 start / stop，在本地聚合为一次活动区结果，skipped（待确认、源码缺失）由桌面端在发起前判定，每项记录到活动区并保留应用名和跳过原因，不合并成一条被后续消息覆盖的提示。

展示组件只接收数据并发出用户动作，不独立管理运行状态或排序。磁贴按安装展示顺序渲染，启动结果、失败、日志刷新不改变顺序。日志刷新仅在显式操作时读取当前安装的日志窗口，保留刷新时间与离线反馈；命令行组件在原型中只演示项目目录与系统终端入口，不提供嵌入式 shell，也不旁路核心执行启动命令。正式 `desktop_terminal(installation_id)` 从当前登记解析存在的项目目录，macOS 使用系统 Terminal 的目录打开能力；不拼接 shell 文本、不提交命令，不改变 AppHost 的运行环境基线。其他平台尚未接入时明确返回 UNSUPPORTED_PLATFORM。

### 文本声明编辑与确认

`manifest read <installation_id>` 返回实际项目根目录、当前来源和路径、原始文本、文件 SHA-256 与安装 revision。`manifest review <installation_id> [--file <buffer>]` 只读校验编辑文本（省略文件时复核当前文件），返回当前文本/拟保存文本、历史确认快照/新声明、校验问题、执行摘要和基线；不生成新的持久化评审文件。调用方以两份文本展示 diff，以完整新声明确认执行内容，保留自己的编辑缓冲。

`manifest save <installation_id> [--file <buffer>] --base-sha256 <hash> --revision <n> --confirm <launch_sha256>` 对应 `manifest-save`。省略文件表示确认当前外部修改，不重写文件。核心先校验项目身份、声明、确认摘要和基线；在线时由安装队列先停止实例，再在登记锁内重新检查 revision 与文件摘要，发布用户原始文本，最后保存确认。文件或登记变化返回 EDIT_CONFLICT，不覆盖编辑缓冲；文件发布后登记确认失败返回 MANIFEST_SAVED_UNCONFIRMED，并给出实际文件摘要，新的执行声明不能借旧确认启动。完成后保持停止，不自动运行。

直接文本以原样字节发布，核心沿用 1 MiB 文件上限；通过 daemon 发送的整个请求沿用 64 KiB 限制，过大请求在停止或写入前拒绝。较大清单可使用外部编辑器修改后执行无文本的确认请求。确认记录新增确认 actor（human / agent / system）和时间；旧确认缺失的归属保持未知，不追填。


## 登录自启与后台测量

`login show` 只读，缺少配置时返回 enabled=false；`login enable|disable` 是显式设置，返回 activation=next_login，下一次登录生效。macOS 在用户 LaunchAgents 目录保存按规范化 data-dir 的 SHA-256 区分的 plist，ProgramArguments 仅调用当前绝对可执行路径、--data-dir 与 daemon serve；RunAtLoad=true，不声明 App 的启动命令或 KeepAlive 重启策略。设置不调用 bootstrap/bootout，不改变当前 daemon 或 App；关闭只移除本工具带来源标记的对应 plist，未知文件冲突不覆盖。首次桌面设置仍要求用户明确选择，不因程序安装或读取设置自动开启。可执行文件移动后需显式重新启用以更新路径。平台不支持时返回 UNSUPPORTED_PLATFORM。

`examples/measure_idle.py` 针对独立 data-dir 和可用的显式网关端口启动 release daemon，预热 5 秒后默认采样 60 秒；合并 daemon 及所有后代进程的 RSS 和累计 CPU 时间，进程集合变化时拒绝宣称完整测量，输出系统、硬件、二进制与版本。脚本只产生 JSON，不写测试报告；网络字节未纳入该脚本，输出明确标记，不能据此判定完整资源验收通过。


### 核心失败建议与诊断预览

`failure::advice(record, consecutive, managed_owner)` 返回 code、level、title、impact、evidence、actions、clears_when。level 使用 blocking / warning / suggestion / record；失败建议为 blocking，附属线索为 suggestion。`failure::read` 在响应的 advice 中合并当前已确认服务的有限项目线索，CLI、RPC 和桌面端共用这一结果。每份建议只有一个 primary=true 的主要操作，动作 ID 为稳定标识，argv 使用数组；所有动作都要求用户显式操作。受管端口占用优先 stop_owner_and_start，执行时仍由既有接口重核占用者安装和实例身份。外部占用先 retry，重复后 edit_manifest；EARLY_EXIT / UNKNOWN 首次完整重启，重复后 preview_diagnostics。DEPENDENCY_MISSING 首次在明确存在唯一包管理器锁文件时提供 copy_dependency_command，重复后 view_reason；没有明确命令则查看证据，不猜包管理器。依赖命令永远 copy_only，不自动执行。

分类在有限 stderr 窗口内不区分大小写，保留原始匹配片段和行号，识别 EADDRINUSE、address already in use、Vite 的 Port <有效端口> is already in use，以及常见项目 CLI（vite、next、tsx、nuxt、webpack、astro、vue-cli-service）的 shell not found 诊断。结构化预检与身份错误不受日志覆盖。仅在已确认服务的受限 cwd 检查固定的 package.json、锁文件、pyproject.toml、requirements.txt 与 node_modules / .venv；目录缺失只返回提示，不改变分类，不增加扫描索引。多个锁文件的包管理器冲突时不生成命令；未观测的运行版本不猜测。

`failure <id> --export [--failure-id <uuid>]` 与 failure RPC 的 export=true 返回 JSON 预览，和 --watch 互斥。sections 数组按 application、command、environment、category、evidence、output_tail、last_success 排序。命令标注为 current_confirmed_declaration_not_historical_argv，不用当前清单冒充失败时命令；现场失效、采集未完成、窗口缺口和当前环境读取失败都保留标记。共享 PATH 和正文再次进行尽力脱敏，不包含 env 原值；preview_required=true，复制和外发由用户决定。omitted 明确列出历史 argv、未探测版本、依赖指纹、浏览器 Console/Network 和留存窗口外输出等缺失内容。读取不创建目录、启动服务、探测版本或发送网络请求；没有现场目录保持 NOT_FOUND。

### Agent 交付接口

`apphost agent-guide` 不依赖 data-dir，输出 version、manifest、troubleshoot、project_prompt 四项 JSON；文本编译进二进制，离线安装时仍与版本一致。源文件为 agent/skills/manifest.md、agent/skills/troubleshoot.md、agent/project-prompt.md。项目提示词可粘贴到 AGENTS.md 或 CLAUDE.md，但命令不自动修改项目文件。

`list --redacted [--all]` 是安全身份投影，只返回 installation_id、project_id、registration_state、requires_launch_confirmation、redacted；省略用户标签、路径、命令、完整清单和 env。普通 list 保留 M0 输出形状。Agent 用 status 读取运行观察、list --redacted 发现登记，不读取普通 list 用于分享。修改清单后 validate --check-launch，再请用户确认；不代为安装依赖、确认声明或 start/stop/restart/stop-all。

### 本地图标

`icon <installation_id>` / `icon` RPC 返回 Icon 或 null，固定顺序检查 icon.png、public/icon.png、public/favicon.png、favicon.ico、public/favicon.ico；不递归、不联网、不使用 SVG。每文件至多 256 KiB，PNG 签名及 IHDR 尺寸检查为 1–1024；ICO 目录至多 32 项，偏移和长度须在文件内。链接文件、越界路径、非法候选不使用，最终由前端首字回退。Icon 包含项目相对 path、mime、sha256、bytes。Tauri 通过 `icon::read(data,id,path,sha256)` 重新校验候选和摘要后取得字节，不直接打开前端提交的任意路径；摘要变化要求重新发现。光栅解码失败同样回退，不把头部检查宣称为完整图片解码。

### 日志保留设置

`log-retention show` / `log-retention` RPC 只读返回 schema_version=1、days、max_mib；缺省 7 天 / 每安装 100 MiB，不因读取创建配置。`log-retention set --days <1..365> --max-mib <10..10240>` / `log-retention-set` RPC 完整替换 `<data>/log-retention.json`，原子发布，非法输入保留旧设置；修改记操作记录。不同客户端完整设置按最后一次成功发布为准，不合并部分字段。设置只影响下一次启动或完整重启触发的普通归档清理，不立即删除，不改变活动日志分段上限与 9 MiB 预留，不删除活动或管理权不确定实例，不修改失败现场和 journal 的保留期限。

### 平台边界与通用编译

`platform` 集中进程组、身份与开机标识、监听归属、端口预检、环境、登录自启、退出观察、日志采集器和本地传输适配入口。通用 lifecycle、retention、events、failure capture、gateway 和 RPC session 不以 macOS 为编译条件。LocalStream 接口提供 authenticate/read_frame/write_frame；各平台适配器负责同用户身份与有界 IO 截止时间，再交给通用分发、订阅和日志跟随。macOS 使用 Unix socket；Windows 命名管道的安全与采集描述符传递仍随 G3 实现，不能替换为无鉴权 TCP。

监听就绪适配区分启动期连接验证与运行期只读监听观察；假平台测试不创建服务，验证共同规则与假本地传输上的 Stop All 持久化恢复。Windows CI 编译全部生命周期并运行通用库、协议、登记冒烟和交付接口测试；这不等于 Windows 服务运行验收。Windows 的进程启动、采集器、自启和公开 daemon 入口在 M1-W 前保持 UNSUPPORTED_PLATFORM。


### 桌面导入与声明确认

`desktop_scan()` 通过原生目录选择器选择根目录，取消返回 null；沿用核心默认扫描限额、登记源解析和启动变更注记，读取过程不写项目或登记。Rust 只保留一个不超过 1 MiB 的临时 Draft，返回 token 与展示副本，不保存额外草稿文件；前端提交 token、分组选择、候选序号及用户编辑的声明文本，不接受前端提供的根路径或来源证据。分组变动更换 token，导入消费 token，旧请求不可重放。候选初始不勾选，用户选择后展示服务 argv、工作目录及完整清单；分组建议必须明确合并或分别登记，合并调用核心 `accept_group`。缺少有效声明的推断候选在桌面提供最小 TOML 模板，可填写多个服务的参数数组、工作目录、端口及浏览器入口，不要求转到 CLI，也不猜启动模块。

`desktop_candidate_read(token,index)` 返回新候选的 TOML 模板；无服务时提供空 command 数组，必须由用户补全。`desktop_candidate_review(token,index,text)` 在核心校验文本、路径边界和原 project_id，成功后替换内存草稿并轮换 token；声明 v1 同步提升草稿版本。只清除已解决的推断与声明校验问题，保留输入读取和来源证据问题；失败不改旧草稿，前端保留编辑。已有项目或外置清单不能通过导入覆盖；扫描后新增清单或改动发现输入仍由核心拒绝。编辑期间禁止导入，用户校验应用后重新勾选并确认完整声明，最终 import 才写文件，不执行命令。

`desktop_import(token, selected)` 仅在明确确认后调用核心 import，按所选声明设置确认摘要，核心仍在写入前检查来源证据、身份和路径边界。已有清单不覆盖，导入不启动服务。桌面只接收安装身份、路径、创建文件及警告的回执，不把 confirmed_services/env 再混入应用列表。失败保留 completed 结果，刷新登记列表，并要求重新扫描后继续，不自动重发旧草稿。草稿限制为 1 MiB。

`desktop_manifest_read/review/save` 复用核心文本编辑契约。读取返回 revision、文件摘要、原始文本；复核展示上次确认、当前文件与拟保存内容，读取以来的文件或登记变化须重新读取后处理，编辑缓冲不会被错误响应覆盖。继续编辑立即使旧复核失效。保存提交复核对应的 revision、base_sha256、confirmed_launch_sha256 和原始文本；有运行记录且后台离线时要求用户显式连接，不由 GUI 使用 current_exe 隐式拉起后台。有运行实例时跟踪 operation_id，接受回执不代表保存成功，停止或保存失败保留缓冲；断连不自动重发。用户可以结束等待并重新读取核对。完成后保持停止，不自动启动。详情标签切换保留缓冲，切换应用或关闭带未保存编辑的详情须明确丢弃。

### 桌面详情读取与复制

详情分为概览、日志、命令行和失败现场，复用原型样式。`desktop_logs(installation_id, instance_id)` 固定请求该实例最近五分钟的日志窗口，正文上限 64 KiB，复用 logs RPC 且不拉起 daemon。打开日志页或点击刷新时读取，不随运行快照轮询。显示观察时间、窗口范围、服务与 stdout/stderr，并保留核心返回的截断、轮转、序号缺口和采集不完整提示。离线或刷新失败时保留同一实例的上次内容并撤销实时描述；切换安装或实例后不沿用旧内容。文本以转义后的纯文本展示，不解释 ANSI 或 HTML。

`desktop_failure(installation_id)` 直接读取核心的最新失败现场与 advice，离线可用，不在前端重算类别或连续次数。`desktop_diagnostics(installation_id, failure_id)` 固定现场身份，复用核心 export 的有序 sections、omitted 和脱敏标记；用户先预览，再点击复制。复制的是当前已预览文本，不在复制时重新生成另一份报告。`desktop_copy(text)` 最多复制 1 MiB 纯文本，只开放写入，不读取剪贴板，不发送网络请求。依赖建议仅复制，不执行；重试走已有显式 restart。edit_manifest / review_manifest 转到同一声明编辑器；其他尚未接入的建议保留核心操作说明及建议命令，明确标为命令行操作，不伪装成已执行。

### 桌面环境与日志保留设置

管理侧以外观、运行环境、日志保留三个页面呈现，复用现有配色；页面首次访问时读取，切换管理页面或返回磁贴保留编辑缓冲，不自动保存、采集或刷新。失败建议 `view_environment` 转到运行环境页并选中对应安装。

`desktop_environment_read` 只读核心 Baseline；`desktop_environment_save(path)` 原样提交手动 PATH，沿用核心绝对目录、大小与数量校验，并清除旧采集来源。`desktop_environment_capture(shell)` 仅接受 zsh / bash，映射固定 `/bin/zsh` / `/bin/bash`；用户展开说明后明确点击“执行采集并保存 PATH”，才执行交互式登录脚本并保存结果。核心负责 5 秒 / 64 KiB 上限、进程组清理及并发保存校验。失败保留原配置和编辑缓冲，不自动重试；成功展示新的采集来源。上述修改仅影响后续启动。`desktop_environment_check(installation_id)` 使用已保存环境调用核心 LaunchPlan::prepare，返回安装身份和服务名、工作目录、可执行文件；不执行命令，不返回 env 原值。PATH 尚有未保存编辑时先保存再检查，结果明确区分检查通过与应用启动。

`desktop_retention_read/save(days,max_mib)` 复用核心日志保留完整设置与边界，界面限制整数且展示生效时机；保存不触发清理或服务操作。环境和保留设置写入沿用 CLI 操作记录动作与 Human 身份，业务保存成功但 journal 写入失败时将 warnings 单独展示，不误报保存失败或自动重发。设置任务在阻塞工作线程执行，读写不隐式连接后台；平台不支持环境设置时只影响该页面。

画布直接输入搜索需保留输入法合成：首个 keydown 为 `keyCode=229` 或 `isComposing` 时仅聚焦搜索框，不追加字符、不阻止默认合成；输入框内的 composition/input 由浏览器处理。
