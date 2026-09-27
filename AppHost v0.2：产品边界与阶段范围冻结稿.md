# AppHost v0.2  
## 产品边界与阶段范围冻结稿

**定位：总体设计与阶段边界，不维护实现进度。**
**目的：定义 AppHost 为什么存在、为谁解决什么问题、负责到哪里、明确不做什么，以及 M0/M1/M2 分别验证什么；各阶段的具体契约见阶段稿与接口稿。**

---

# 1. Product Story：为什么要有 AppHost

AI Coding / Vibe Coding 让个人开发者可以快速产生大量小型应用。

这些项目通常具有共同特点：

- 前后端分离或基于 localhost；
- 启动命令、端口和依赖各不相同；
- 有些半年都不会再打开；
- 项目数量远高于传统手写项目；
- 很多项目“能跑”，但缺乏长期维护能力；
- 一旦环境、依赖、浏览器或运行方式变化，重新启动和定位问题的成本很高。

开发时常见的工作流是：

```text
找到项目目录
→ 打开终端
→ 回忆启动命令
→ 启动 backend
→ 启动 frontend
→ 找正确端口
→ 打开浏览器
→ F12
→ 看 terminal stderr
→ 把错误复制给 AI
```

这套流程对于一个项目并不严重。

问题在于，当个人拥有十几个、几十个由 AI 快速产生的 App 后：

> **人开始忘记项目，而项目仍然存在。**

AppHost 要解决的不是“怎么创建更多 App”，而是：

> **让已经存在的 App 在几个月之后仍然容易找到、启动、理解和调试。**

因此 AppHost 的核心故事不是“更强的开发平台”，而是：

> **AI 写完 App 之后，负责让它继续活着。**

---

# 2. 两条产品契约

v0.2 只保留两条最高级产品原则。

## 2.1 不用为了运行和调试 App 打开终端

沿用此前的可证伪标准：

> **安装 AppHost 后，一个月内，不需要为了启动或调试自己的 App 手工打开终端。**

它不是要求用户永远不能使用终端，而是：

> AppHost 不应该因为自身能力缺失，迫使用户重新回到终端完成日常运行和基本故障定位。

原始边界分析也将这一点作为所有功能取舍的核心判断标准。

---

## 2.2 为遗忘而设计

AppHost 假设用户三个月后已经忘记：

- 这个项目是做什么的；
- 应该怎么启动；
- 哪个目录才是正确版本；
- 上次运行是否正常；
- 为什么最后一次失败；
- 当时使用了什么运行环境。

因此：

> **用户无需重新阅读源码，就应该能够重新理解并启动一个旧 App。**

AppHost 不是为“记得住所有项目的开发者”设计的。

它是为：

> **未来已经忘了今天做过什么的自己**

设计的。

---

# 3. 最重要的边界：Ownership

所有功能决策首先经过 Ownership 判断。

只存在三类对象。

---

## 3.1 Project-owned

属于项目和用户：

```text
source code
Git repository
package.json
pyproject.toml
lockfile
app.toml
AGENTS.md
项目数据库
业务配置
```

AppHost 默认只能：

```text
read
inspect
validate
observe
```

AppHost **不应该默认修改或删除这些内容**。

例如：

- 不自动 `git pull`；
- 不自动升级依赖；
- 不自动修改源码；
- 不自动删除项目；
- 不因为检测到缺依赖就擅自执行安装。

如果需要修改 Project-owned 状态，应交给用户、IDE 或 Coding Agent。

---

## 3.2 AppHost-owned

由 AppHost 自己产生：

```text
registry metadata
runtime metadata
logs
event timeline
browser profile
cache
failure record
snapshot metadata
```

AppHost 对这些数据拥有完整生命周期管理权：

```text
create
rotate
archive
clean
purge
```

---

## 3.3 External-owned

由操作系统或外部工具拥有：

```text
Node
Python
pnpm
uv
Git
Chrome
Docker
OS runtime
```

AppHost 可以：

```text
detect
inspect
invoke
compare
```

但不成为：

```text
新的 nvm
新的 pyenv
新的 Homebrew
新的 Docker Desktop
```

---

## 3.4 Ownership 是功能裁决器

以后面对任何新需求，先问：

> 它属于谁？

例如：

**自动安装 Node？**

External-owned → 不做。

**自动删除源码？**

Project-owned → 不做。

**清理 AppHost 创建的浏览器缓存？**

AppHost-owned → 可以做。

这一规则优先于任何“顺手也能支持”的功能扩张。

---

# 4. AppHost 的职责范围

AppHost 的职责只保留四层。

```text
L3  AI Interface
      ↓
L2  Observation
      ↓
L1  Runtime
      ↓
L0  Registry
```

层级关系沿用此前的四层模型；上层可以依赖下层，下层不能反向要求 AI 才能正常工作。

---

## 4.1 L0 Registry：这个 App 是什么

负责：

- 注册 App；
- 保存 Manifest；
- 保存稳定身份；
- 保存人类可读标签；
- 保存项目路径；
- 保存最后运行时间和最后成功状态；
- 检测 Source 是否仍然存在。

Registry 首先服务于：

> **让用户重新找到和理解自己的 App。**

---

## 4.2 L1 Runtime：让它跑起来

负责：

- 启动；
- 停止；
- 重启；
- 管理子进程；
- 捕获 stdout / stderr；
- 分配或检查端口；
- 提供稳定入口；
- 必要时按需唤醒；
- 在 daemon 重启后重新确认真实运行状态。

L1 不负责：

- 升级源码；
- 回滚源码；
- 安装开发环境；
- 修改系统网络；
- 优化业务代码。

---

## 4.3 L2 Observation：告诉用户发生了什么

负责：

- 收集运行事件；
- Browser Console / Network 等调试信息；
- 后端 stdout / stderr；
- 启动和退出事件；
- 基础健康状态；
- 将不同来源的信息按 App 和时间对齐；
- 保留失败现场。

L2 的核心不是：

> 收集更多日志。

而是：

> **告诉用户这些信息属于哪个 App、哪一次运行，并尽可能解释它们之间的先后关系。**

此前边界稿已经指出，前端 Browser 信息和后端进程日志的身份绑定与时间轴对齐，是相对于普通 Browser MCP 更重要的差异点。

---

## 4.4 L3 AI Interface：把已有信息交给 AI

负责：

- Diagnostic Capsule；
- 后续 MCP；
- 后续 Skill；
- 将 L0–L2 已经拥有的信息结构化输出。

L3 不负责：

- LLM 对话；
- Agent Loop；
- Coding；
- 自动决策；
- 自动修改源码。

原则：

> **AI 可以使用 AppHost，但 AppHost 不依赖 AI 才能存在。**

---

# 5. 身份模型

Pre-M0 只冻结三层身份。

```text
project_id
    ↓
installation_id
    ↓
instance_id
```

---

## project_id

一个逻辑项目。

---

## installation_id

该项目在当前机器上的某个具体 checkout / 工作目录。

例如：

```text
~/Projects/pdf-reader
~/Projects/pdf-reader-test
```

属于不同 Installation。

---

## instance_id

一次：

```text
start → running → stop/crash
```

对应一个 Instance。

Restart 后必须产生新的 Instance。

---

## 人类标签

机器身份不能代替用户认知。

每个 Installation 还必须具有：

```text
human_label
```

默认可以来自：

```text
目录名 + 分支名
```

但应该允许修改，并保证当前界面中尽量不出现无法区分的同名项。

用户看到的是：

```text
PDF Reader
PDF Reader · test
```

而不是 UUID。

---

# 6. 状态模型

不再采用十几个生命周期状态。

只保留两个正交状态机和一个保留策略字段。

---

## 6.1 Registration State

```text
REGISTERED
SOURCE_MISSING
DETACHED
```

其中：

- `REGISTERED`：源码位置正常；
- `SOURCE_MISSING`：路径失效，需要重新关联；
- `DETACHED`：不再由 AppHost 管理。

---

## 6.2 Runtime State

```text
STOPPED
STARTING
RUNNING
STOPPING
FAILED
```

这五个状态足以覆盖 M1。

崩溃原因、用户停止、daemon 重启等属于：

```text
event / exit_reason
```

而不是额外状态。

---

## 6.3 Retention Policy

```text
active
dormant
archived
```

它控制：

- 日志保留；
- Browser Profile 保留；
- Failure Record 保留；
- AppHost-owned cache 清理策略。

`purge` 是 Command，不是生命周期状态。

---

# 7. Manifest：只描述稳定事实

自动发现只发生在首次导入。

```text
folder
↓
scan
↓
draft
↓
user confirmation
↓
manifest
```

之后：

> **Manifest 是运行时唯一声明真相源。**

自动扫描不能成为每次启动重新猜测项目结构的机制。此前方案也已将自适应发现收敛为首次导入的草稿生成器。

---

## 首次导入只要求用户确认一件事

> **“这个 App 应该怎么启动？”**

其余内容全部：

- 自动推断；
- 使用默认值；
- 或等真正需要时再补充。

不能让用户为了注册一个 App 填十几个字段。

---

# 8. 批量导入是首要体验

AppHost 面对的典型用户不是只有一个项目的人。

首次运行应该支持：

```text
选择 ~/Projects
       ↓
发现 23 个候选 App
       ↓
预填启动命令
       ↓
用户选择候选并复核
       ↓
一次注册
```

导入必须满足：

> **批量选择、明确确认、可解除登记。**

猜错不是严重问题。

猜错的项目在第一次启动时进入 Failure UX，并在那里自然修正。

不要要求：

```text
23 个项目
×
23 次配置向导
```

---

# 9. M1 的核心产品其实是 Failure UX

App 正常运行时：

```text
● Running
```

所有 Supervisor 都差不多。

AppHost 是否有价值，主要发生在：

> **App 没跑起来的三十秒。**

因此 M1 至少识别六种 Failure：

| Failure | 基础识别 |
|---|---|
| PORT_IN_USE | bind / EADDRINUSE |
| DEPENDENCY_MISSING | 常见 module/package 缺失 |
| RUNTIME_MISMATCH | Manifest/环境提示与实际版本明显冲突 |
| INVALID_COMMAND | command/script 不存在 |
| EARLY_EXIT | 启动后短时间退出且未形成服务 |
| UNKNOWN | 无法归类 |

---

## Failure 页面首先告诉用户“发生了什么”

例如：

```text
PDF Reader 启动失败

原因：
端口 5173 已被占用

占用进程：
node · PID 18273

[Retry]
```

或者：

```text
Backend 启动后 1.2 秒退出

最后输出：

ModuleNotFoundError: fastapi
...
```

用户不应该先：

```text
Open logs
→ 搜 stderr
→ 猜哪个错误重要
```

---

## 不越过 Ownership

检测到：

```text
pnpm install
```

可能解决问题时，默认行为是：

```text
Missing dependencies detected

Suggested command:
pnpm install

[Copy]
```

而不是直接执行。

只有未来 Manifest 明确声明某个安全的 bootstrap command 后，才可以进一步提供执行能力。

---

# 10. Diagnostic Capsule

M2 前期最重要的 AI 接口不是 MCP，而是：

> **Copy Diagnostic Context**

当 App 失败时，一键生成自包含文本：

```text
App
Human label
Project path
Start command

Runtime environment
Recent fingerprint change

Failure category

stderr excerpt

Recent timeline

Previous successful run
```

用户可以直接粘贴给：

- ChatGPT；
- Claude；
- Codex；
- Issue；
- 群聊。

这样能够首先验证：

> **AppHost 组织出的上下文本身是否足够好。**

如果 Capsule 对 AI 没有帮助，增加 MCP 也不会解决本质问题。

---

# 11. Correlation 不承诺不存在的精确性

默认情况下：

```text
Browser Request
Gateway Event
Backend stdout
```

无法保证拥有统一 Trace ID。

因此观察结果需要允许：

```text
correlation = exact
```

或者：

```text
correlation = inferred
```

默认关联依据：

- 同一个 Instance；
- 时间邻近；
- Endpoint；
- 状态码；
- 事件先后。

如果未来某个 App 主动集成 AppHost Middleware，再提供 Exact Correlation。

Pre-M2 不冻结跨组件 Trace Protocol。

---

# 12. Gateway 的边界

M1 的 Gateway 只为了：

1. Stable Endpoint；
2. Port Broker；
3. On-demand Wake。

因此只保留：

```text
Port Broker
Endpoint Registry
Local Gateway
```

不进入：

```text
Interface Inventory
VPN management
System DNS
Network Policy
Advanced traffic analysis
Production reverse proxy
```

网络能力仍然属于 Runtime 实现机制，而不是独立产品方向。

---

# 13. apphostd 自身故障语义

Supervisor 自己也会崩溃。

因此 M1 必须保证：

> **daemon 重启后，不相信磁盘上遗留的 RUNNING 状态，而是重新确认真实世界。**

启动后执行：

```text
load registry
↓
inspect previous instances
↓
process still exists?
endpoint still alive?
↓
reconcile
↓
rebuild actual Runtime State
```

如果无法确认某个遗留进程身份：

> 不擅自杀死，也不假装已经接管。

至少记录：

```text
stale / orphan evidence
```

并告诉用户。

M1 暂时不承诺复杂的 daemon crash takeover，只承诺：

> **状态最终能够收敛到真实世界。**

---

# 14. Event Retention

Observation 如果在用户看到之前已经被滚掉，就没有价值。

M1/M2 初始默认：

```text
Structured Timeline:
7 days

Raw stdout/stderr:
Default: 7 days or 100 MiB / installation
Configurable in M1: 1–365 days, 10–10240 MiB / installation
whichever comes first

Failure Records:
last 20 failures
up to 30 days

AppHost Action Journal:
30 days
```

具体数值未来允许根据真实使用调整。

但 Failure 发生时，需要单独固定一个最小现场：

```text
failure 前 30 秒
failure 后 10 秒
stderr tail
```

普通日志轮转不能立即删除它。

---

# 15. Trust Contract

AppHost 是一个：

- 开机运行；
- 能创建进程；
- 能停止进程；
- 能观察浏览器状态；

的常驻程序。

因此信任必须成为 M1 功能。

---

## Stop All

必须存在一个显眼的：

> **Stop All Managed Apps**

它首先是心理安全机制。

---

## Action Journal

所有 AppHost 主动执行的行为都可查：

```text
wake
start
stop
restart
route change
cleanup
```

用户永远可以回答：

> **“AppHost 刚才做了什么？”**

---

## Uninstall / Detach 文案

界面必须明确告诉用户：

> **AppHost 不拥有，也不会因为注销或卸载而删除你的项目源码。**

---

## 系统通知

M0/M1：

> **默认完全不发送系统通知。**

未来只有经过真实使用验证确有必要的通知才加入。

---

# 16. Lightweight Contract

“轻量”必须能测量。

M1 工程目标：

```text
Idle RSS < 50 MB

Idle CPU
< 0.1%
1 minute average

Idle network traffic
≈ 0

Managed apps running by default
0
```

原则：

> **用户没有操作时，AppHost 应尽可能接近不存在。**

任何需要 AppHost 空闲时长期维持 Chromium、Node、Python 等额外 Runtime 的设计，都应被重新审查。

---

# 17. 首页只服务两个高频意图

真实用户打开 AppHost 时最常见的是：

### “把 X 打开”

和：

### “X 为什么坏了”

因此主界面只需要：

```text
Search
```

和一个 App 列表。

排列原则：

- 磁贴按分组与安装顺序固定位置，失败和启动不会重排。
- 失败与待处理通过角标、计数和筛选体现。
- 最近使用是可关闭的小号分组；只在应用较多时出现，其内部按最近使用排序，其他磁贴保持原位。具体阈值与交互见 M1 阶段稿。

低频的：

```text
所有历史项目
归档
高级配置
```

进入二级页面。

AppHost 第一版不设计“专业运维 Dashboard”。

---

# 18. 可测用户体验目标

以下作为产品验证目标，而非永久性能承诺。

### First Value

安装 AppHost → 第一个 App 成功运行：

> **< 3 分钟**

---

### Failure Explainability

启动失败中，不要求用户打开原始日志文件就能理解基本原因：

> **> 70%**

---

### Forgotten App Recall

三个月未使用的 App：

> **100% 可以不读源码就知道它是什么、最后一次何时运行、最后状态如何。**

---

### Idle Cost

```text
RSS < 50 MB
CPU < 0.1%
```

---

### Notification Noise

日常使用：

> **≈ 0 系统通知**

---

# 19. M0：只冻结“App 是什么”

M0 不写复杂 Runtime。

## 交付

### Manifest Schema v0

只包含 M1 确实需要的信息。

### Identity Model

```text
project
installation
human_label
```

### Validator

能够判断 Manifest 是否满足启动最低要求。

### Batch Import

能够扫描父目录并生成 Import Draft。

---

## M0 验证标准

使用至少：

```text
3 个不同技术栈真实 App
+
一批存量项目目录
```

验证：

1. Manifest 不别扭；
2. 用户首次导入只需确认启动方式；
3. AI 可以稳定生成 Manifest Draft；
4. 批量导入不会要求逐个配置；
5. 错误推断可以轻松撤销和修改。

M0 结束前不冻结：

```text
Capability Model
Public Event Protocol
MCP API
Adapter ABI
Snapshot Schema
```

---

# 20. M1：证明“不用终端”

M1 是第一个真正产品阶段。

## 交付

```text
apphostd

Registry

Process Supervisor

Port Broker
Endpoint Registry
Local Gateway

stdout / stderr

Runtime State

daemon reconciliation

Failure Taxonomy

Stop All
Action Journal

Minimal UI
```

---

## M1 用户可以完成

```text
找到 App
↓
Open
↓
自动启动
```

失败时：

```text
Open
↓
Failure Page
↓
直接知道基本原因
```

而不是：

```text
打开 Terminal
```

---

## M1 验证标准

连续一周真实使用：

> **没有因为“启动 App”而必须手工打开终端。**

这一阶段验证逻辑延续此前“做完 Runtime 后必须通过实际使用验证基本假设”的原则。

如果一周后仍然经常绕过 AppHost 手工启动：

> 停止增加功能，重新检查产品假设。

---

# 21. M2：证明“调试上下文有价值”

M2 才开始强化 Observation。

## 范围

```text
Structured Timeline

Browser console/network observation

Backend log correlation

exact / inferred correlation

Failure Record

Diagnostic Capsule

Experimental Snapshot
```

---

## M2 不要求

```text
完整 APM
完整性能分析
完整 MCP
Flutter Inspector
WebKit 深层适配
跨浏览器兼容矩阵
```

---

## M2 验证标准

选择真实 Bug：

> **用户不需要手工整理前端和后端日志，仅使用 AppHost 输出的 Diagnostic Context，就能够让 AI 定位到正确的问题区域。**

此前方案已经指出，只有进入前后端关联和 AI 可消费的可观测上下文后，产品才真正超过普通启动器。

---

# 22. M2 之后再决定

只有获得 M0–M2 实现经验以后，才讨论：

```text
Capability Model
MCP stable API
Snapshot stable schema

Electron deep integration
Tauri/WebView integration
Flutter/Dart VM integration

Advanced performance context

LAN/VPN exposure

Plugin SDK

Remote management

Automatic compatibility analysis
```

这些不是被永久否定。

只是：

> **现在没有足够事实支持设计它们。**

---

# 23. Explicitly Not Now

Pre-M2 明确不做：

```text
公网部署

容器编排

完整 Network Manager

VPN 管理

生产级 Gateway

系统 DNS 管理

完整性能/APM系统

自动修改源码

自动 git pull

自动依赖升级

自动环境安装

自动 rollback

源码文件管理器

删除用户项目

Agent Runtime

内置 AI Chat

共享写入数据库

通用 Browser Automation

复杂 Adapter Plugin System

完整 Flutter / WebKit 抽象
```

如果开发过程中再次出现这些需求，默认回答是：

> **Not now.**

除非它阻塞 M0/M1 的核心用户故事。

---

# 24. 最终职责定义

AppHost 最终负责的是：

> **记住一个 App 是什么，让它能够再次运行，并在失败时保留足够上下文帮助用户理解发生了什么。**

在 Pre-M2 阶段，它不需要证明自己：

- 支持所有框架；
- 能自动修 Bug；
- 能优化性能；
- 能接管系统网络；
- 能管理所有开发环境。

只需要证明两件事：

### 1.

> **我不再需要记住怎么启动这些小 App。**

### 2.

> **它坏的时候，我不用重新从零寻找发生了什么。**

如果这两件事成立，AppHost 才值得继续长出 MCP、Snapshot、更多 Runtime Adapter 和更深入的 AI 协作能力。

---

# 25. v0.2 冻结项

进入代码设计之前，暂时只冻结：

```text
Product Contract

Ownership Model

L0 → L1 → L2 → L3 dependency direction

project_id
installation_id
instance_id

Registration State

Runtime State

Retention Policy

Manifest v0 scope

M0 scope

M1 scope
```

其余设计允许在实现过程中修改。

**不因为文档已经写过，就把未经实践验证的抽象当成兼容性负担。**

---

## 一句话总结

> **AppHost 是一个为遗忘而设计的本地 App Supervisor：记住 App、把它跑起来，并在它坏掉时留下足够清晰的现场。**

v0.2 到这里应停止继续扩产品边界。

各阶段按自己的验收条件推进，不因后续功能已经实现而跳过真实使用验证。