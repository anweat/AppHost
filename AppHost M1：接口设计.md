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

## 已登记安装的启动准备

`registry::launch_snapshot(data_dir, installation_id)` 只读取得当前项目内生效清单，验证登记未解除、源码根目录仍为登记的规范化目录、project_id 一致、声明结构有效且确认摘要匹配。缺失登记返回 NOT_FOUND，解除登记返回 DETACHED，源码不可用返回 SOURCE_MISSING，确认缺失或行为字段改变返回 REVIEW_REQUIRED。历史确认快照只用于核验，不能在清单缺失时充当启动声明。

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

每段有容量上限；创建新段后按最老段清理，原始文件和对应索引一起清理。缓冲、队列及索引都必须有界；超长行和二进制数据不引发无界分配。普通留存按安装总计 7 天 / 100 MB，失败现场另存，不能仅对单个 stdout 文件应用整个安装的预算。

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

协议版本为 1。请求 request_id 为非空、最多 128 字节的可见 ASCII 字符串，method 最多 64 字节，params 必须为对象；拒绝未知信封字段。短 RPC 每连接一条请求和一条响应，然后关闭；后续订阅使用单独连接。请求正文最多 64 KiB、响应正文最多 1 MiB，以换行结束；不完整帧、无效 UTF-8/JSON、大小超限明确失败。响应携带 version 和 request_id，以及 result 或 error 二者之一；无法识别请求时 request_id 为 null。版本不兼容返回 PROTOCOL_VERSION_UNSUPPORTED，不静默降级。

同时最多接纳 16 个 RPC，超额连接立即关闭；客户端收到 DAEMON_UNAVAILABLE，可显式重试。单次请求读取和响应写入各有 2 秒绝对期限，缓慢逐字节发送不能刷新期限。阻塞的文件预检在有限工作线程中执行，不占用接受连接的主循环；无请求时主循环阻塞等待，不轮询。

`hello` 参数为空对象，返回 protocol_version、daemon_id（每次进程启动重新生成）、pid、data_dir 和 capabilities。客户端先完成同版本握手再发送其他 RPC。`check-launch` 仅接受 installation_id，返回与 LaunchPlan::summary 相同的预检结果，不执行服务、不持有可供客户端复用的启动授权。只有 capabilities 声明的已实现方法可以调用；未知方法返回 METHOD_NOT_FOUND。运行操作、事件订阅和历史状态查询按下述契约接入，不能用空状态响应冒充已完成对账。

### 生命周期与持久化

显式 `start / stop / restart / stop-all` 在 daemon 不在线时，可以用当前 AppHost 可执行文件拉起 `daemon serve`，在 5 秒内完成同版本握手；启动错误提示用前台 serve 查看原因，不另选 data-dir 或端点。`status` 不拉起 daemon：在线返回 `live: true`，离线只读取历史并明确返回 `live: false`。`logs` 只读且不拉起 daemon。运行方法必须出现在 hello.capabilities 中。

`start / stop / restart` 的回执包含 installation_id、instance_id、operation_id、operation_done 和当前 state。回执表示请求已经接受；调用方通过 `status` 等待同一 operation_id 的 operation_done，读取 operation_error 或 operation_result。客户端断连不取消已接受的操作。重复 start 合并同一 STARTING / RUNNING 实例，不重新执行声明；新实例仍须使用 LaunchPlan 完成当前确认核验。FAILED 且仍有服务、采集器或不确定身份时，start 返回 RESTART_REQUIRED。

运行快照保存在 `<data>/runtime/<installation_id>.json`，schema_version 为 1。记录安装、实例和操作身份、五态状态、各服务的安全日志键、声明端口与有效超时、spawn 意图、进程组身份、存活/就绪观察及退出证据；采集器身份单独记录。未取到退出码时明确记录 exit_status_unavailable。Runtime State 仅由 daemon 写入，采集器不参与。原子发布需要同步文件和父目录，启动意图及 spawn_attempted 必须先于相应系统 spawn；拿到 OS 身份后再发布身份检查点。发布中断留下的专用前缀临时文件不参与对账，取得单例锁后只清理可识别的未发布普通文件；损坏的已发布快照明确报错。runtime 仅识别规范 UUID.json，instances 仅识别规范 UUID 目录，日志根仅识别规范十进制段号；以点开头或名称不符合约定的目录项跳过，每次扫描合并一条结构化警告，不删除、不写入 stderr。离线 status 在顶层 warnings 返回本次扫描提示；在线 status 顶层返回本次 daemon 对账提示，实例 warnings 返回其启动清理提示；日志读取在 window.warnings 返回本次扫描提示。提示采用 code / level / title / evidence / actions / clears_when，仅表示对应扫描时的观察，下次同类扫描覆盖，不改变运行状态。每次最多扫描 4096 个目录项；名称合法但内容、类型或身份不合法的权威记录仍报错，符号链接不能成为记录或日志段。

每个安装最多一个工作线程，最多同时管理 32 个活动实例、每实例 32 个服务、1024 个安装快照。不同安装的同步启动派发使用 daemon 内互斥锁串行执行；启动路径获取登记文件锁最多等待 2 秒，获得锁后重新复核快照，超时返回 BUSY，不自动重新执行操作。其他登记命令保持原有非阻塞语义。日志归档清理、采集器就绪及登记文件锁等待均在派发锁外。取得登记锁并完成复核后才获取派发锁，仅覆盖同步 spawn 及其身份检查点；排队时不持有安装快照锁或全局安装表锁。派发锁不承载可变数据，单个安装 panic 后允许恢复这把锁，不据此恢复损坏的安装记录；就绪期限从派发完成起计。不同安装独立等待就绪及停止，RPC 不等待这些长操作；相同安装的 Stop 与同步 spawn 派发在短临界区内排序。先停止再重启的后半段重新准备当前声明并预检端口，不复用旧命令。同步 spawn 返回错误后停止派发尚未启动的服务，关闭未使用管道；已经启动的服务保留为失败实例的诊断对象。

就绪检查仅连接声明的 loopback 端口，并通过 macOS libproc 的监听 socket 信息及当前进程身份，证明监听者属于该服务进程组。其他进程抢先监听不能满足就绪。无端口服务连续存活 2 秒后只表示进程就绪；声明的启动期限耗尽返回 READINESS_TIMEOUT。启动时每约 100 ms 检查一次；进入 RUNNING 后每约 1 秒用 libproc 被动核验监听归属，不再向服务建立探测连接。进程组消失或观察到服务退出仍判失败。运行中端口丢失时保留 RUNNING、ready=false 作为就绪告警；从首次观察丢失起按单调时间给予 10 秒宽限，恢复后清除告警和计时，连续满 10 秒后记录 PORT_NOT_READY。已经失败的实例不因端口恢复而自动恢复 RUNNING。停止意图仍以约 100 ms 的轻量检查响应，未到观察时间不刷新进程组、不枚举 FD、不序列化记录。端口预检分别试绑 IPv4/IPv6 的通配与 loopback 地址，共四处，以覆盖 macOS 允许可复用通配地址与具体地址并存的行为；保留 SO_REUSEADDR，避免把已经关闭服务的 TIME_WAIT 当作活动监听。任一地址占用即拒绝；预检不保留端口，实际启动仍须核验监听归属。占用返回 PORT_IN_USE，其他试绑错误返回 PORT_CHECK_FAILED。冲突结果包含可取得的占用 PID/名称；占用者查询最多扫描 65536 个 PID、约 250 ms，IPv4/IPv6 合并一次 FD 枚举，找到一个代表性监听者即结束。结果 scope=first_match，owners 至多一项，found 表示已取得占用证据；incomplete 仅表示未找到且本次查询受限，不承诺列全所有监听者。limitations 分别记录 permission_denied、unavailable、timed_out、pid_limit 与 pid_list_failed；界面优先展示已找到的进程，找不到且受限时才提示信息不可得，该结果不授予停止外部进程的权限。

停止各服务先发一次 SIGTERM，等待各自声明期限，再发一次 SIGKILL 并有界核验。只有进程组清空后才回收 Child；新启动但瞬时退出的组长可由 `capture_spawned(&Child)` 在尚未回收时记录身份，不能借此认领未知后代。采集器在服务停止后有 2 秒处理 EOF，仍未完成时显式结束并标记日志不完整。停止期间采集错误不能取消 STOPPING。所有已知组清空且没有未解决的 spawn 身份缺口时，才进入 STOPPED；停止失败不执行重启或后续登记变更。

恢复时先拿单例锁，核验全部已有身份，再接受新启动。系统启动时间已改变的旧组视为已消失；同次启动中非空组必须有匹配的已知活成员。磁盘上的 RUNNING 不能直接恢复为 RUNNING，需要重新验证就绪；仍未就绪的进程进入有期限检查。缺失身份的 spawn 意图、身份不一致或不能完成的核验保留为 FAILED + management_uncertain，不认领、不发信号、不重复启动。没有存活进程的 FAILED 实例仍保留 FAILED 与失败原因，直到显式停止或启动改变状态，不因 daemon 或系统重启消失。已接受的停止、完整重启及停止后的登记变更，其意图随运行快照保存；恢复后仍先停止旧组，成功后才执行后半段。

Stop All 先在版本化 wake.json 中一并持久化 automatic_wake_paused 和递增的取消代数，安装操作记录接受时的代数。恢复时旧代数的活动实例仍须停止，旧代数的重启后半段不得继续；这覆盖全局意图已发布而安装记录尚未更新的崩溃窗口。新的显式操作使用当前代数，不因自动唤醒仍暂停而被拒绝。该代数只用于 Stop All 取消，不替代每安装独立的登记 revision。随后取消尚未派发的启动和重启后半段，向各安装独立提交停止；返回逐安装操作回执或错误，最终结果从对应快照读取。显式 start 只授权本次运行，不解除自动唤醒暂停。

wake.json 缺失且存在运行记录时，以最大 operation_epoch 加一恢复取消代数并保持自动唤醒暂停，取消未完成的重启后半段，继续已持久化的停止意图，保留正常运行的服务。先逐条保存统一代数和取消后的意图，最后发布 wake.json；中途再次崩溃时仍按缺失文件恢复，不将此次归一化解释成 Stop All。没有运行记录时初始化为未暂停、代数 0。代数溢出、已有文件损坏或版本不支持仍明确报错，不猜测内容。

活动或有运行历史的安装，CLI 的 detach / relink 经 daemon 协调；离线时先拉起并对账。协调请求成功停止后才调用 L0 登记写入，长操作的登记结果位于 operation_result。无 daemon、无运行记录的 M0 登记操作仍使用原 L0 入口和回执。批量 detach 在提交任何停止前核验所有安装 ID，并去重，未知 ID 不产生部分停止或登记写入。relink 的 RPC 路径必须是绝对路径，由 CLI 在发送前规范化；停止旧实例不依赖当前源码或清单仍可读取。

### 实例日志读取与普通归档

每实例的独立采集器继承父进程分配的有限描述符列表，一次读取全部服务的 stdout/stderr；不提供由管理 RPC 注入描述符或任意日志路径的入口。服务名只显示，文件使用 s0、s1 等安全键。日志位于 `<data>/instances/<instance_id>/logs`，每段 1 MiB、最多 8 段，包含原始数据和观察索引。采集器正常 EOF 写完成标记；服务主动关闭输出流而继续运行不算采集故障。

`logs` 窗口输入为 installation_id、instance_id、from_ms、to_ms、limit；时间为观察时间的闭区间，limit 为原始字节预算，1–131072，默认 65536。响应包含安全键与服务名映射、原始字节块及窗口缺口标记、实例 log_incomplete。JSON 字节数组会膨胀，仍受统一 1 MiB 响应上限。日志读取在独立 RPC 中进行，不占用生命周期工作线程；后续跟随读取不能阻塞停止。

新实例替换已完成实例前，保存旧实例的只读 record.json，历史日志仍按 installation_id / instance_id 校验归属读取。启动和完整重启时清理过期或超量的普通归档：安装预算为 100 MiB，给当前实例预留 9 MiB，旧归档按创建时间淘汰并应用 7 天年龄上限；不删除活动或管理权不确定的实例。扫描上限 4096 个实例目录、15000 个归档条目；异常目录、链接或超限明确失败，不猜测归属。普通日志归档不承载 D9 的独立失败现场。

### 独立失败现场

`failure` 接受 installation_id 和可选 failure_id；不指定 failure_id 时读取最近一次失败。CLI `apphost failure <installation_id> [--failure-id <uuid>] [--json]` 直接读取持久化现场，不启动 daemon 或 App。响应包含 failure、history、details_expired 和连续同因观察次数。没有现场目录时返回 NOT_FOUND；已知现场超过保留期限时返回 failure=null、details_expired=true，保留最近失败的摘要。损坏的权威记录明确失败，不当成空历史。

现场位于 `<data>/failures/<installation_id>/<failure_id>.json`，schema_version 为 1。记录包含安装、实例（预检失败可以为空）、服务、phase（preflight/startup/runtime）、观察时间、底层 cause_code、分类 evidence、脱敏详情、日志快照、stderr_tail、collecting 与 gaps。evidence 带 code、rule_id、inferred、匹配片段和所检查 stderr 文本中的行号；具体依赖、端口或版本错误优先于泛化退出，运行中的普通退出归 UNKNOWN。运行状态中的底层失败原因继续保留，分类结果不改写生命周期。

daemon 使用一条独立的有界采集队列（64 项），最多保留 32 个待补尾窗口；空闲时阻塞等待。生命周期不等待失败后的 10 秒，容量或 IO 错误通过 status.warnings 报告 FAILURE_CAPTURE_INCOMPLETE。首先保存可读现场，再异步补齐观察区间 `[失败时间-30秒, 失败时间+10秒]`。初次与最终日志分别标记 initial/tail，可能重叠，不伪装成去重后的连续日志。每次日志快照读取至多 64 KiB 原始字节，stderr 独立保留至多 32 KiB 的最新字节用于分类，以免 stdout 洪泛挤掉报错；展示的 stderr 尾部至多 8192 字符。跨采集块的文本先按服务和流合并，再匹配和脱敏。

环境确认快照中的值、凭据特征行和本机用户路径默认脱敏；任意程序输出无法保证完全识别，返回 redaction 提示供分享前复核。JSON 转义和脱敏可能扩大正文，最终现场受 900 KiB 上限约束，必要时截断输出并记录 evidence_byte_limit，为 1 MiB RPC 信封预留空间。读取、轮转、采集器退出和 daemon 恢复的缺口均显式标记。daemon 离线且补尾期限已过时，读取将未完成的 collecting 显示为 false，并添加 tail_capture_not_completed，不永远显示采集中。

写入时按每安装最多 20 条、30 天清理，读取不创建目录、不清理文件。最后失败引用与最近成功时间保存在独立 history.json 中，普通日志被清理不影响已保存的现场；迟到的补尾不能重新插入已被较新失败淘汰的记录。连续同因次数从上次成功之后的留存记录计算；达到留存边界时 count_limited_by_retention=true，调用方只能将该值显示为下界，不能声称完整历史次数。

运行状态仅 STOPPED / STARTING / RUNNING / STOPPING / FAILED；就绪、退出原因和 management_uncertain 为附加信息。操作 ID 用来关联长请求，不另建持久化任务状态机。

请求采用 JSON Lines：`{version, request_id, method, params}`；响应回显 request_id，成功返回 result，错误返回沿用 M0 的 `{error:{code,message,details}}`。长操作返回 operation_id 后通过状态查询/事件获得结果；断连不取消已接受的操作。事件游标与快照绑定，过期游标要求重新读取快照。

| 方法 | 输入 | 行为 |
|---|---|---|
| start | installation_id | RUNNING 幂等；STARTING 合并；FAILED 仍有存活服务时要求 restart；启动前核验当前声明 |
| stop | installation_id | 可取消 STARTING；退出已证明归属的全部进程后才报告 STOPPED |
| restart | installation_id | 先停止旧实例，成功后重新校验并分配新 instance_id |
| status | installation_id 可选 | 返回当前状态、活动服务与提示；daemon 离线不将历史视为实时 |
| logs | installation_id、instance_id、from_ms、to_ms、limit | 有界观察窗口；返回服务映射和缺口标记，慢读不阻塞 stop |
| url | installation_id | 只读解析稳定入口，不拉起 daemon 或 App |
| failure | installation_id、failure_id 可选 | 返回结构化失败、证据和现场完整性 |
| stop-all | 无 | 暂停自动唤醒、取消待启动、停止所有活动服务，返回逐项结果 |
| journal | cursor、limit | 有界读取操作日志 |

网关固定端口、浏览器唤醒条件和 Windows Job 生命周期按阶段稿的验证要求确定；这里不先定义兼容回退组合。平台未支持能力明确返回 UNSUPPORTED_PLATFORM。


## 桌面端交互适配（设计契约）

界面展示模型由登记信息与运行观察按 installation_id 关联生成，不在 ui.json 维护一份运行状态。没有运行记录的已登记安装可显示为未启动；live=false 或连接中断的记录明确标记为历史观察。待确认、源码缺失和目录扫描警告作为提示，不扩充五态枚举。启动和停止按钮使用 daemon 操作回执，跟踪同一 operation_id 的完成和错误；迟到的旧回执不得覆盖新的用户意图，Stop All 后不得由前端计时器重新启动应用。

列表和详情消费 warnings、失败原因、服务 ready/alive 及操作结果。结构化警告不等于失败，不根据 stderr 是否非空推导运行状态；CLI 的成功 JSON 查询不向 stderr 输出警告。失败详情的端口信息以 ownership.found 决定是否展示监听者，以 limitations 解释查询受限；scope=first_match 不代表唯一占用者。

桌面入口只承接用户的显式操作，复用核心的 scan/review/import、LaunchPlan 和运行管理契约，不在界面重新实现源码探测、声明确认或进程管理。真正的系统目录选择、核心声明校验、浏览器稳定入口、失败现场与操作记录接入分别沿对应设计实现；交互原型不替代这些能力的验证。首次桌面实现前再确定前端构建工具和 Tauri 桥接细节，不由无依赖 HTML 设计原型反向决定产品技术栈。

桌面反馈（冻结稿 5.9、10.4）需要接口提供以下数据，前端只做比较和展示，不自行推断或累计：

- 失败记录：code、phase、service、证据（含 rule_id 与是否推断）、输出末尾及匹配行位置；PORT_IN_USE 的占用者归属（AppHost 安装的 installation_id，或外部进程的名称与 PID）；READINESS_TIMEOUT 的已证明归属实际监听端口。
- 连续同因次数：自上次成功以来，同一 code 与 service 的连续失败次数，由 daemon 从失败记录计算；成功后清零。
- 条件解除：端口占用类提示按 clears_when 推送"占用已解除"的观察（受管占用者停止、外部占用者消失）。只在桌面端订阅且提示存活期间检查，频率和时长有上限；解除后只改为提供重试，不自动启动。
- 自上次成功以来的变化：成功时记录分支、声明摘要及其确认来源（发起者）和运行环境基线版本（冻结稿 9.5），失败详情与当前值比较；没有记录的项不显示，不猜测。
- 启动进度：STARTING 期间提供 started_at 与有效 startup_timeout_ms，用于显示已等待时间和期限。
- "停止它并启动"作为一个复合操作（D10），回执区分停止占用者与启动两个阶段。
- 批量结果：stop-all 逐项返回 stopped、failed（原因）、canceled（取消的待启动）和 orphan（身份无法确认、未结束的进程及证据）。分组只是 ui.json 的外观数据，daemon 不感知分组：分组启停由桌面端对各安装分别发起 start / stop，在本地聚合为一次活动区结果，skipped（待确认、源码缺失）由桌面端在发起前判定。

展示组件只接收数据并发出用户动作，不独立管理运行状态或排序。磁贴按安装展示顺序渲染，启动结果、失败、日志刷新不改变顺序。日志刷新仅在显式操作时读取当前安装的日志窗口，保留刷新时间与离线反馈；命令行组件在原型中只演示项目目录与系统终端入口，不提供嵌入式 shell，也不旁路核心执行启动命令。正式终端桥接随桌面接口设计确定。
