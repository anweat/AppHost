# AppHost M0：接口设计

## 范围与入口

依据《AppHost v0.2：产品边界与阶段范围冻结稿》第 19 节，实现 Rust 核心库和 CLI。
M0 负责声明、校验、批量发现、确认登记及解除登记。实际启动和运行检查属于 M1。端口继续由清单明确声明，M0 不增加猜测；Windows 的 npm.cmd/bat 在 M1 启动器中需要明确的 shell 调用与参数转义策略，当前 PATH 检查不表示可以直接 spawn。
不引入导入任务状态机、后台服务、HTTP API、数据库或动态插件机制；发现规则仅提供 Rust 源码接口。

项目声明文件固定为 `apphost.toml`。旧分析稿中的 `app.toml` 仅为早期名称，不作为兼容输入。
确认导入是创建声明文件的明确授权；扫描和校验只读。已有声明文件不自动覆盖。

M1 运行层复用这些登记接口时，登记持久化按《AppHost M1：接口设计》采用 Registry v1 和每安装 revision：旧 v0 只读兼容，首次登记写入备份并迁移；项目内 Manifest v0 与 ImportDraft v0 的声明和确认语义保持兼容。

## Manifest v0

```toml
schema_version = 0
project_id = "ef42ed4a-b42b-45a6-92fa-069a0e997bd1"
name = "Paper Reader"
description = "本地论文阅读器"

[[services]]
name = "backend"
cwd = "."
command = ["uv", "run", "uvicorn", "backend.main:app", "--host", "127.0.0.1", "--port", "8000"]
port = 8000

[[services]]
name = "frontend"
cwd = "web"
command = ["npm", "run", "dev", "--", "--port", "5173"]
port = 5173

[services.env]
NODE_ENV = "development"
```

| 字段 | 约束 |
|---|---|
| schema_version | 必填，整数 0；不兼容版本报错 |
| project_id | 必填，非零 UUID；由首次草稿生成，随项目复制保留 |
| name | 必填，非空可读名称 |
| description | 可选，默认空字符串 |
| services | 至少一项；每项是独立启动声明，不表示依赖或启动顺序 |
| services[].name | 必填、非空、同一 Manifest 内唯一 |
| services[].command | 必填，字符串数组；第一项是非空可执行程序，其余为参数；不隐式调用 shell |
| services[].cwd | 默认 `.`；相对项目根目录，必须存在且为目录，不允许绝对路径、父目录穿越或符号链接逃逸 |
| services[].port | 可选，1–65535；只声明预期端口，不分配端口或测试占用 |
| services[].env | 可选，字符串映射；变量名不可为空或含 `=`，名称及值不可含 NUL；不读取 `.env` |

拒绝未知字段，避免拼写错误被忽略。声明不承诺命令存在、依赖已安装或服务能够启动。
M0 不定义 instance、运行状态、服务依赖、健康检查协议或外部凭据引用。
写入清单时省略空 env，服务字段按 name、cwd、command 排列，command 保持单行 TOML 数组。

## 身份与登记

`project_id` 属于项目；每个规范化的实际目录在本机获得独立的 `installation_id`。
同目录通过符号链接再次导入视为同一次安装；不同 checkout 可以共享 project_id。
不根据 Git remote、目录名或代码相似度猜测项目身份；路径移动通过 relink 显式处理。
无清单副本可通过 `associate DRAFT PATH INSTALLATION_ID --output NEW_DRAFT`，明确复用已登记安装的 project_id。
只修改草稿身份；已有不同身份的 apphost.toml 不覆盖。相同 remote 不构成自动合并依据。
CLI scan 读取可用的本机注册表，将项目 name 与已登记项目的当前清单名称比较，notes 中给出可能副本的安装 ID 和路径；
每个候选最多显示 8 个匹配。同名只是线索，不更改身份或默认选中，不读取 Git remote。
这项可选提示最多使用 8 MiB 且不超过扫描剩余读取预算；来源不可读、摘要或身份不符时不猜测，提示不完整时明确说明。

安装记录包含 `installation_id`、`project_id`、绝对 `path`、`human_label`、`detached`。
默认 human_label 只保存稳定目录名与消歧后缀，label_is_custom 标识显式命名；草稿改名和 label 命令都视为显式命名。
list 实时读取项目自身 Git HEAD，返回 current_branch（游离 HEAD 使用短提交）；自动名称的 display_label 附加当前分支，手动名称保持原样。
旧记录无法证明名称来源，缺省按手动名称保留，不按字符串猜测并删除后缀；current_branch 始终表示当前读取结果。
Manifest 的 name 仍是项目名称，两者独立。重名时只附加目录名与数字序号，不附加绝对路径。
手动改名若与其他未解除安装重名则返回 CONFLICT；重新登记已解除记录时也重新检查显示名称冲突。
同一路径重复确认保持安装身份不变；路径上出现不同 project_id 时返回冲突。
对同一路径重新登记已解除的记录，恢复原安装身份。

Registration State 由事实计算：detached 为真时为 DETACHED；目录不存在时为 SOURCE_MISSING；
否则为 REGISTERED。目录访问失败不伪装成 SOURCE_MISSING，而是返回错误。
声明损坏或丢失另以诊断字段返回，不增加生命周期状态。
不保存 Runtime State、最后运行时间或假定的成功结果。
安装记录保留 `retention_policy = active | dormant | archived`，旧记录缺省为 active。
M0 仅存储该策略，不实现日志、缓存清理或运行状态切换。
安装可记录 `created_manifest: {sha256}`，表示 AppHost 曾实际创建 apphost.toml 时的原始字节摘要。
旧记录和无法证明来源的已有清单为 null，不反推创建者；重复导入不刷新原始摘要。
CLI import 的每项额外返回 `created_files`，只列本次实际新建的文件及摘要。发布成功但登记失败时也在错误详情返回该信息；
没有成功保存的来源信息无法在下次导入时凭文件内容恢复，后续按未知来源处理，不增加恢复日志。
CLI detach 的每项额外返回 `retained_files`，列出记录过创建来源的文件绝对路径、原始/当前摘要和匹配情况；始终不删除文件。

登记数据保存在 AppHost 自有目录下的 `registry.json`。全局 `--data-dir` 或环境变量
`APPHOST_DATA_DIR` 可覆盖默认路径；默认 macOS 为 `~/Library/Application Support/AppHost`，
Windows 为 `%LOCALAPPDATA%/AppHost`，其他平台为 `$XDG_DATA_HOME/apphost` 或 `~/.local/share/apphost`。
写操作使用操作系统文件锁串行化并原子替换文件；进程退出即释放锁，不自建持久锁状态机。

## CLI 契约

所有业务输出为 JSON；错误为 `{ "error": { "code", "message", "details" } }`，写入 stderr。
字段错误统一为 `{ "field", "message" }`。成功退出码 0，业务失败 1，命令行参数错误 2。
`--help` 使用普通可读文本。

| 命令 | 输入与输出 |
|---|---|
| `apphost scan PATH [--max-depth 3] [--output draft.json]` | 扫描目录，输出 ImportDraft；depth 0 只检查 PATH 本身；output 指定时仅新建，不覆盖 |
| `apphost validate PATH` | PATH 为项目目录或声明文件；输出解析后的 Manifest；不执行命令 |
| `apphost import draft.json --yes` | 导入 selected=true 的条目；`--yes` 明确确认已审阅草稿；没有它不写入 |
| `apphost list [--all]` | 返回安装记录、即时注册状态和声明诊断；默认不返回已解除记录 |
| `apphost detach INSTALLATION_ID` | 解除登记，重复执行无副作用；不删除项目或 Manifest |
| `apphost detach INSTALLATION_ID ...` | 批量解除登记，所有 ID 先校验，再一次保存；可使用导入输出的 installation_id 列表，不维护批次状态机 |
| `apphost review draft.json [--output reviewed.json] [--all] [--describe]` | 交互确认合并建议与未选候选；指定 --output 才保存草稿，stdout 不输出 JSON；默认不逐项询问已选且未发生启动声明变更的候选；--all 可重新确认它们 |
| `apphost associate DRAFT PATH INSTALLATION_ID --output NEW_DRAFT` | 将草稿中的指定目录关联至已有逻辑项目，保留独立安装 |
| `apphost label INSTALLATION_ID LABEL` | 修改本机显示名称，不修改项目声明 |
| `apphost relink INSTALLATION_ID PATH` | 为源码失效的安装关联新目录；目标必须有同 project_id 的有效清单且未被其他安装占用；保留安装 ID、名称和保留策略 |
| `apphost retention INSTALLATION_ID POLICY` | 修改保留策略，不改变注册状态或项目文件 |
| `apphost validate PATH --check-launch` | 输出 `{manifest, warnings}`；只读检查可执行文件与直接的包管理器 run 脚本，不执行命令；警告不阻止登记 |

典型调用：

```sh
apphost scan ~/Projects --output /tmp/apphost-draft.json
apphost review /tmp/apphost-draft.json --output /tmp/apphost-reviewed.json
apphost import /tmp/apphost-reviewed.json --yes
apphost list
apphost validate ~/Projects/example
apphost detach <installation_id>
```

## ImportDraft 与发现规则

```json
{
  "schema_version": 0,
  "root": "/absolute/projects",
  "candidates": [
    {
      "path": "/absolute/projects/demo",
      "selected": true,
      "human_label": "Demo",
      "manifest": { "schema_version": 0, "project_id": "UUID", "name": "Demo", "description": "", "services": [] },
      "evidence": [{ "path": "package.json", "sha256": "原始文件摘要" }],
      "issues": []
    }
  ],
  "issues": []
}
```

示意中的空 services 需要补齐才可导入。issues 是扫描诊断，不是持久状态；导入时重新校验实际输入。
草稿不保存隐藏会话，可由用户或 AI 修改后交回 CLI；不可把未经审阅的外部草稿直接确认。
草稿和待解析的配置输入上限 1 MiB。锁文件仅流式计算 SHA-256，使用 64 KiB 缓冲，单文件上限 16 MiB。
扫描默认深度 3、最大深度 16；单次最多访问 10,000 个目录、枚举 100,000 个目录项、读取 64 MiB 元数据。
`--max-directories`、`--max-entries`、`--max-read-bytes` 可以收紧这些预算，不能提高硬上限；
`--exclude NAME` 可重复追加要排除的目录名，不支持 glob 或跨目录路径。超出总预算返回 `SCAN_LIMIT`，不输出貌似完整的部分草稿。
Draft 的 `scope` 回传本次扫描范围；树遍历和规则判断使用同一个总读取预算。

声明优先，内置发现规则如下：

1. 已有 `apphost.toml`：直接解析与校验；错误时保留候选且默认不选中，不降级猜测。
2. `package.json`：优先 `scripts.dev`，其次 `scripts.start`；生成包管理器 run 命令。
   使用 packageManager 的 npm/pnpm/yarn/bun 声明，其次使用唯一锁文件指示，均无时用 npm。
   包管理器矛盾、未知包管理器或无可识别脚本时保留候选并取消默认选中。
   带 exports，或同时带 main 和 files 时视为可能的库/工具；直接使用 tsc、tsdown、tsup、rollup、esbuild 且含 -w/--watch/--watch= 参数时视为可能的构建监听。保留建议命令，但要求确认，不据此断言项目一定不是 App。此检查不解析任意 shell 脚本。
3. `pyproject.toml`：只有一个 `[project.scripts]` 入口时预填该命令，但默认不勾选；入口可能是普通 CLI，必须确认具体启动方式。
   多入口或无入口时保留候选，等待填写启动声明，不猜 ASGI 模块名。只有 requirements.txt 时也作为 Python 候选，启动命令留空。
4. 同目录同时匹配多条规则但无 apphost.toml 时，保留为未选中的候选，各条建议放入 `suggestions`，提示用户明确选择或组合。
   不以规则注册顺序决定优先级，不自动把文档、开发工具或命令行入口解释成运行服务。

扫描跳过 `.git`、`node_modules`、`.venv`、`venv`、`target`、`dist`、`build`、`__pycache__` 和目录符号链接。
发现候选后保留一个根候选，在全局深度范围内最多继续向下两层，寻找嵌套项目的配置文件名。
嵌套结果放入 `nested_projects`；除下述待确认归组建议外，不读取其配置正文、不生成独立安装或合并服务。每个候选最多展示 32 条线索，
达到数量预算时设置 `hints_truncated`；用户可缩小根目录单独查看。已有 apphost.toml 的项目不展开嵌套线索。
前后端多服务或其他技术栈可以通过用户编写的 apphost.toml 表达；下面的归组建议须经确认。
对于没有根候选且直属子目录构成 frontend/backend、web/server、web/api 或 client/server 的两个推断候选，
扫描额外生成 `groups` 合并建议。配对目录只需在本次范围内实际访问过，无识别标记的一侧使用空命令服务。
分组根目录等于扫描根目录时不建议合并，但命中配对的推断成员仍须确认；要查看单个 App 的合并建议，应从它的父目录扫描。
配对外的其他候选具有完整且有效的启动声明时，将此父目录视为项目集合，不提出合并建议；只取消配对的推断成员勾选并备注可能遗漏服务，其他候选保持原状。
只有唯一配对可建议归组；多个配对互相竞争时不猜选择，但仍对各配对涉及的推断成员取消默认选中并提示。其他兄弟候选保持原状，已有清单继续拥有自己的边界。
配对备注同时说明手工路径：确认 App 的共同根目录后，在该目录编写 apphost.toml，以多个 [[services]] 及各自 cwd 声明服务，再重新扫描；不将项目集合根目录自动视为 App 根目录。
内置服务目录名按 ASCII 大小写不敏感匹配，真实路径保持原样；大小写不同但同时存在的匹配目录仍视为竞争边界。配对成员目录下隔一层的候选（如 Frontend/Vue）同样取消默认勾选并提示；此情形不自动提出合并，不扩大扫描深度，也不影响无关兄弟候选。
已有清单不参与推断归组。接受建议只替换 members 中明确列出的候选，绝不按路径前缀删除其他候选；剩余后代与合并根同时勾选时，import 的重叠路径预检要求用户选择边界。
建议中的服务 cwd 加上子目录前缀；缺失命令保持空值等待确认，两个原候选取消默认选中，避免只登记前端。
`review` 接受建议后才替换原候选，生成父目录的一个 Manifest；拒绝时可独立确认原候选。
合并候选的 `source_dirs` 记录直属来源目录；根目录与各来源的配置证据均须在导入前复核。
根候选下只有一个直属子项目线索且名为 web/frontend/backend/server/api/client 时，也生成根服务与子服务的合并建议。为生成该建议读取该子目录的配置正文，
计入同一读取预算；已有清单或嵌套线索被截断时不推断该边界。根目录和子目录均可能是文档或开发工具，必须明确确认。
任何推断候选只要带有嵌套线索（包括已访问的上述常见子目录），都取消默认选中并提示可能遗漏服务。
归组只使用扫描范围内已经发现的目录，不增加树扫描深度；未来归组规则扩展不等于动态插件加载。
Node、Python、命名归组判断分别位于 rules/node.rs、rules/python.rs、rules/grouping.rs；核心边界与草稿合并放在 discovery/groups.rs。
GroupingRule 提供 is_service_directory(name) 和 sibling_pair(已访问的直属目录名集合)，不获得遍历或读写句柄。scan_with_policies 可替换发现规则集与归组规则；核心始终执行扫描根、项目集合、已有清单、成员范围、预算和显式确认约束。返回集合外成员或重复成员属于 INVALID_INPUT。
可选 sibling_pairs 方法默认包装 sibling_pair，保持社区实现兼容；内置规则返回所有已命中的配对，供不合并时的成员提示使用。核心最多接受 16 个配对，不以配对顺序决定归组优先级。
扫描只读取上述声明、锁文件，不执行项目脚本、不安装依赖、不读取提示词或密钥文件。
目录不可读等局部错误进入 issues，不中断其他目录；根目录不可读则整次失败。

list 返回当前有效 manifest（包含 name、description、services 启动声明）、current_branch、display_label 和静态启动警告。
每次确认导入为所有已选清单保存 confirmed_launch_sha256 和 confirmed_services 启动声明快照，包括项目自带的清单。快照只用于下次复核比较，不作为运行依据或历史日志；旧记录缺少快照时明确显示旧命令未保存，不从当前文件反推。摘要为带版本域的规范 JSON SHA-256，覆盖 schema_version、project_id 和完整有序 services（name/cwd/command/env/port）；不覆盖项目名称、描述、TOML 排版和注释。
注册表 v0 中的快照固定按 Manifest v0 校验历史摘要。Manifest 升级到 v1 前，须设计版本化快照及迁移，不能直接用当前版本解释旧快照。confirmed_services 保存 env 原值，可能包含敏感信息；它是 AppHost 数据中的额外副本，后续导出须脱敏，数据清理须涵盖该副本。M0 不提供导出或自动清理功能。
list 返回 current_launch_sha256、launch_changed（无旧摘要或无法校验时为 null）和 requires_launch_confirmation；缺少确认、声明变更、清单无效或不可读均不得视为确认。
scan 读取注册表，已登记路径的当前摘要与确认摘要不一致（或旧记录缺少确认摘要）时取消默认勾选，并展示新旧启动声明。
review 在显式同意后为候选写入 reviewed_launch_sha256，绑定本次声明内容；修改声明后失效。CLI review 同样刷新登记比较，不能因旧草稿仍勾选而跳过。
import 在整批写入前核对注册表；上述变更缺少匹配的 reviewed_launch_sha256 时返回 REVIEW_REQUIRED，单独 --yes 或改 selected=true 不足以重新确认。未选项仍跳过；首次导入和未变更的重复导入保持原有批量确认。
草稿仍可由用户编辑，复核摘要防止普通流程误确认，不提供针对恶意伪造草稿的密码学证明或隐藏会话。静态启动检查仍是警告，用户在 review 中明确看到并同意后可以确认；缺少本机运行环境不升级为禁止登记。
重新 scan → review → import --yes 才刷新确认摘要；list、label、relink、detach 都不刷新。M1 必须在实际启动前重新读取、校验和比较同一份声明，不得使用 list 的历史结果放行。摘要确认的是启动声明，不保证脚本内容或依赖版本不变。
review 的交互和简短摘要写入 stderr；每个服务只显示名称、cwd 和命令，非空 env/port 附加展示；不向 stdout 输出草稿 JSON。仅指定 --output 才保存草稿，否则只复核预览，不保存确认结果。
已有 apphost.toml 的待确认候选（含 --all）只询问 y/N，不提供命令编辑入口，并提示“如需修改，请直接编辑 apphost.toml 后重新扫描”。无现有清单的推断候选仍可编辑待新建清单的命令。拒绝确认的候选保持未选，不影响其他已选候选导入。
review --describe 可在已选候选或本轮确认的候选描述为空时询问用途，回车跳过；默认批量流程不额外询问。已有项目清单通过项目文件修改描述，review 不覆盖已有清单。

## 确认、重复导入与失败语义

导入前先对整批已选条目进行校验：版本、规范路径、重复目录、字段、cwd、证据摘要及登记身份。
校验失败时整批不产生项目写入；未选中条目不阻碍其他项目导入。
确认前源声明或锁文件发生变化，返回 STALE_DRAFT，要求重新扫描。
已存在 apphost.toml 必须与草稿 Manifest 语义相同，否则返回 CONFLICT；修正它应直接编辑项目文本。

通过预检后逐个导入，已有记录不重复创建。新 Manifest 采用不覆盖的原子发布，登记文件原子替换。
项目目录和登记目录之间不伪装成跨文件系统事务：出现写入错误立即停止，返回已完成条目及失败路径。
如果 Manifest 已写入但登记失败，保留 Manifest 并明确报告；相同草稿可以重试完成登记。
不会为了回滚而删除项目文件，也不引入后台重试、补偿队列或额外状态机。
证据校验是发现过期草稿的保护，不是面对恶意并发改写项目的安全沙箱。

常用错误码：`INVALID_INPUT`、`VALIDATION_ERROR`、`NOT_FOUND`、`CONFLICT`、`STALE_DRAFT`、
`REVIEW_REQUIRED`、`IO_ERROR`、`REGISTRY_INVALID`、`BUSY`、`SCAN_LIMIT`。JSON 错误包含字段问题；批次失败包含 `completed` 和 `failed_path`。

## 核心库边界

- `Manifest::validate(root)`：对声明及工作目录做边界校验，返回字段问题。
- `scan(root, max_depth)`：使用内置规则和默认预算返回 ImportDraft，不写项目或登记数据。
- `scan_with_options(root, options)`：显式收紧扫描预算和排除目录。
- `scan_with_registry(root, options, data_dir)`：同一读取预算内比较已登记安装的启动确认，并追加可用的项目名称线索；不登记或关联。
- `annotate_launch_changes(data_dir, draft)`：刷新草稿中的登记确认比较，供 CLI review 使用；注册表读取失败明确报错，不假设没有历史确认。
- `scan_with_rules(root, options, rules)`：使用调用者明确提供的完整 Rust 规则列表。
- `import(data_dir, draft)`：预检、确认写入并返回安装记录。
- `import_with_receipts(data_dir, draft)`：同一导入操作额外返回各项本次新建文件，供 CLI 展示。
- `list(data_dir, include_detached)`、`detach(data_dir, id)`、`label(data_dir, id, value)`：最小登记操作。
- `relink(data_dir, id, path)`：验证源码失效和目标身份，仅更新原安装路径。
- `retention(data_dir, id, policy)`：更新静态保留策略。
- `Manifest::launch_warnings(root)`：使用服务 cwd、env.PATH 或进程 PATH 检查可执行文件；检查直接的 npm/pnpm/yarn/bun run SCRIPT。包装器、shell 内部命令、依赖安装与版本匹配留给运行阶段，不宣称静态检查可以证明可启动。

CLI 只解析参数、调用核心库并序列化结果。文件读写、规范路径、原子发布和错误转换集中处理；
只为发现判断提供下述 Rust 源码级扩展点，不建立通用工作流引擎或平台空实现。

## Rust 发现规则接口

```rust
pub trait DiscoveryRule {
    fn id(&self) -> &str;
    fn markers(&self) -> &[&str];
    fn inputs(&self) -> &[&str] { self.markers() }
    fn evaluate(&self, context: &RuleContext<'_>) -> Result<Proposal>;
}
```

`RuleContext` 提供目录显示名、白名单文件内容和文件存在性，不提供遍历句柄；
`Proposal` 包含名称、说明、services 和 `requires_confirmation`。Node/Python 内置规则与社区 Rust 规则使用相同接口。
调用者传入完整规则列表即可添加或替换判断，不需要修改遍历器；最小调用示例为：

```rust
scan_with_rules(root, &ScanOptions::default(), &[&NODE, &PYTHON, &my_rule])?;
```

目录归组使用独立接口，同样由调用者显式替换一套判断，不按规则注册顺序猜优先级：

```rust
pub trait GroupingRule {
    fn is_service_directory(&self, name: &str) -> bool;
    fn sibling_pair(&self, directories: &BTreeSet<String>) -> Option<[String; 2]>;
}
scan_with_policies(root, &ScanOptions::default(), &[&NODE, &PYTHON], &my_grouping)?;
```

核心统一处理：规则 ID 与文件名校验、文件读取和预算、指纹、Manifest 校验、多规则冲突、嵌套线索与默认勾选。
最多注册 16 条规则，合计最多 32 种输入文件；markers/inputs 仅允许简单文件名，禁止路径、glob、`.env` 和 `.env.*`。
`apphost.toml` 保留给核心，优先于所有推断规则。额外输入记录在 Candidate 的 `rule_inputs` 中，导入时同样校验指纹。
scan 为候选和合并建议加入 `launch_check.*` notes；review 在确认前及修改命令后重新检查并展示，已选候选的提示也会显示。
list 返回即时 `launch_warnings`。检查不执行项目命令，警告不阻止登记；scan 内的配置读取仍计入同一预算。
Candidate 的 `notes` 表示需确认的建议，`issues` 表示读取、解析或声明问题；它们都不是持久状态机。

M0 仅支持随 AppHost 编译的可信 Rust 实现。此接口不是第三方代码沙箱或稳定的二进制 ABI；
社区规则的动态加载形式、分发与信任边界留待后续决策，不加入外部程序执行或自动下载。

## 远程扫描验证入口

`cargo run --example github_scan -- [--repo OWNER/REPO] [--search QUERY] [--limit 10] [--max-depth 3] [--subdir PATH]`
是开发侧的扫描实验入口，不属于 M0 的登记命令。`--repo` 可重复，`--search` 通过 GitHub 仓库搜索自动选样；
都不指定时运行代码内的混合样本及非 Web App 对照。显式仓库支持 `--reference` 指定分支、标签或提交。
`--scan-parent` 模拟从项目集合扫描：临时仓库作为扫描根的子目录，使仓库顶层的服务可以产生合并建议。`--max-depth` 仍相对于仓库（或 --subdir）计算，核心扫描深度加一，因此此模式最多设置 15。元数据投影同步读取该模式下归组所需配置，仍遵守原来的文件、字节和树大小上限；输出 scope.scan_parent 标明比较方式。

使用现有 `gh` 登录态，只发送 GitHub API GET 请求。先固定 commit SHA，再读取对应的目录树和所需配置 blob，
不执行 git clone、不下载仓库压缩包、不获取源码全集。必要的目录与配置临时投影到本机，由同一个 Rust `scan`
执行判断，结束后删除临时文件。`--exercise-import` 追加 review → import → list → 重复导入验证，
只接受默认已选声明，所有归组与不明确候选均跳过；注册表和生成清单均在临时目录中，正常完成时显式检查删除成功。
输出包含自动勾选数、待确认数、导入数和各步骤程序耗时；程序耗时不能当作人工审阅耗时。
该入口验证远程仓库的静态识别行为，不声称把执行过程部署到了云主机。

`--subdir` 先解析固定提交下的目标子目录，再请求其目录树，避免为子项目获取全仓库的递归树。
单次树响应上限 12 MiB，树条目最多 20,000、目录最多 10,000；达到限制时要求缩小范围，不猜测缺失内容。
每仓库最多读取 80 个配置文件、8 MiB 配置正文，单请求超时 30 秒；扫描深度仍由 `--max-depth` 限制。
超过扫描器对应读取限制（声明 1 MiB、锁文件指纹 16 MiB）的文件，
用明确标记的超限占位文件复现该边界，不下载其正文。目录树截断、网络错误、无法安全投影的配置符号链接等
返回 `inconclusive`，不能归入“未发现项目”。项目启动、依赖、gitignore 文件和实际运行环境不在验证结论内。

stdout 输出 JSON：`summary`、`transfer`、`results`。每项携带仓库、固定提交链接、候选目录、生成命令、
实际脚本声明、问题以及被父项目截断的嵌套候选目录。结果分为 `auto_selected`、`manual_required`、`mixed`、
`no_candidates`、`inconclusive`；前者仅表示扫描器默认选中，不等于已经证明可运行。
选样数量有限且按 stars 排序时，仅代表便利样本，不以命中比例冒充整体准确率。

退出码 0 表示采集完成且内置负例没有被自动选中；1 表示存在无法判定的仓库或负例误选；2 表示参数或选样失败。
常规单元测试使用离线边界数据，远程实验显式执行；结果只输出到终端，不维护测试报告或阶段记录。

自动推断能力与 M0 验收分开衡量：正确默认勾选率及 60% 目标作为能力指标如实报告；M0 采用每个 App 最多一轮启动确认、没有静默漏服务或静默误登记的交互验收。接受合并并补齐未知命令属于同一轮；需要手写根清单的情况单独计为手动修正，不计作一轮确认通过。元数据实验不能证明实际命令只启动预期服务，也不能替代真实用户对清单可读性与 AI 生成声明的验收。
M1 需验证 webpack-dev-server --open 等启动命令的浏览器副作用与前端单独启动的失败体验。M0 不执行命令，也不擅自移除用户确认声明中的 --open；具体控制方式在 M1 设计中决定。

## AI 生成声明的输入输出约定

`examples/manifests/` 提供 Paper Reader、ComfyUI 和 Kamaen Fieldnotes 的声明示例，分别表达
多服务 Web App、Python 单服务和 Node Web App。它们是接口样例，不是启动成功记录；cwd 相对于各自项目根目录。
示例 UUID 仅用于演示，新逻辑项目生成一次新的 UUID，同一项目的副本复用该身份。

生成规则：

- 输入必须包含用户指定的 App 根目录、可读取的项目配置，以及已确认的启动方式；同仓库或相邻目录不自动证明同一个 App。
- 输出完整 `apphost.toml`；command 为程序和参数数组，每个服务单列 cwd。前后端属于同一个 App 时放在同一 services 列表。
- 命令、端口和服务关联有明确来源才填写；没有证据时询问启动方式，不编造 ASGI 模块、端口或 Python 环境。
- 不增加未定义字段，不读取或复制密钥；不生成安装依赖、升级代码、启动顺序或运行状态。
- 用户确认后用 `validate --check-launch` 检查结构与当前机器的静态提示，再通过 scan/import 登记。

这些样例及本接口的字段表共同定义生成输入；真实运行、M1 入口选择及运行环境匹配仍须单独验证。
