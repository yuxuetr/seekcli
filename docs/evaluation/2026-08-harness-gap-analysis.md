# SeekCLI × DeepSeek Harness 全面对照评估

> 评估时间：2026-08-22
> 被评估方：SeekCLI `main` @ `52dd8da`（阶段十九完成）
> 对照基准：`deepseek-harness`（DeepSeek AI 官方开源 agent harness，`dsh`）
> 评分口径：见 [scoring-rubric.md](scoring-rubric.md)

---

## 0. 结论摘要

| 口径 | 得分 | 判断 |
| --- | --- | --- |
| Harness 引擎内核（L0-L7 运行时机制） | **65%** | 纠偏机制扎实，扩展点缺失 |
| Agent 产品形态（可分发/可扩展/可集成） | **25%** | 基本只能作者自用 |
| **整体** | **约 40%** | 骨架完整、深水区欠账集中在三处 |

**一句话结论**：SeekCLI 在 **L1 运行时纠偏**与 **L7 评估闭环**这两个「真 harness 与裸 ReAct 的分界线」上做得扎实，
认知层面（`AGENT_ARCHITECTURE.md`）甚至可以说清醒；但在 **L2 工具面广度**、**L4 会话数据模型**、**L5 可扩展性**
三处是**架构级**而非功能级的欠账。

**三处架构级欠账**：

1. **L4 会话是状态快照而非事件日志** —— `Session { messages: Vec<Message> }` 直接序列化。
   dsh 的 append-only `SessionEvent` 让 fork / resume / 回放 / UI 保真 / 遥测**全部免费派生**；
   SeekCLI 现在这个结构，上述任何一项都得单独造。**越晚改越贵，是唯一建议推倒重来的地方。**
2. **L2 工具面既窄又封闭** —— 8 个工具，硬编码 `match` 派发，无 MCP。
   第三方无法给 SeekCLI 增加任何能力，只能改 Rust 源码。
3. **L5 一切在编译期定死** —— dsh 的一切在 `cordis.yml` patch 层可换（包括 agent loop 本身）。
   这是「能不能长成生态」的分水岭。

> 注：SeekCLI 原自评「Harness 全景图 12 项组件全部落地」，在**其选定的那张验收清单内**是成立的。
> 但那张清单覆盖的是「单进程 ReAct harness 的纠偏机制」，不覆盖数据模型、扩展性、分发形态。
> 本次换用 dsh 作为坐标系后，分数下修属于**口径变更**，不是能力退步。

---

## 1. 对照基准：deepseek-harness 是什么

| 维度 | 数据 |
| --- | --- |
| 形态 | pnpm monorepo，TypeScript，npm 包 `@deepseek-ai/dsh` |
| 规模 | 2476 个 TS 文件 / **约 56.5 万行**，50+ 包组 |
| 测试 | **696 个 spec 文件**，7 套 vitest 配置（unit / e2e / snapshot / web / web-perf / web-stress / shared） |
| CI | **18 个 GitHub workflow** + GitLab CI，gate 化（`scripts/run-gates.ts`） |
| 过程资产 | `.agents/notes/` **1484 篇设计笔记**（proposed / implemented / rejected / archived 四态），11 个自研 skill |
| 文档 | 每篇 en/zh 配对 + `.i18n.yaml`，5 类目录自动生成并受 CI 新鲜度门禁 |

### 1.1 架构内核

`docs/architecture.md` 的核心主张是 **everything is a plugin**，底座是 Cordis：

- **没有特权 core**。模型适配器、工具注册表、会话日志、**乃至 agent loop 本身**都是可替换插件
  （原文：`dsh-agent-loop` is swappable）。
- **Profile / Bundle 分层组合**。启动时按 bundle 顺序叠加 → profile patch → home patch → `--patch`；
  `dsh --profile web --dump-config` 导出实际启动树，任意一行都能被 patch 替换。
- **Capability Seam 三角色**：Service Definition（接口）/ Service Provider（实现）/ Consumer（通常是模型工具）。
  这是它最值钱的设计——`fs` 与 `subprocess` provider 共享同一个「执行世界」，
  把它们指向远程沙箱，Bash / PTY / LSP **同时**迁移过去，零 provider fork。
- **Model-visible means logged**。任何进入模型请求的东西必须能从 append-only 的 `SessionEvent` 日志重建，
  并有运行时不变量断言。fork / resume / 回放 / 遥测 / 持久化全部从这条流派生。
- **turn / step 生命周期 + waterfall 事件**。`agent/pre-step`、`agent/request`、`llm/stream`、
  `tools/pre-execute|execute|post-execute` 都是可拦截的 waterfall，监听器必须 `next()` 才下传。

### 1.2 模型可见工具面

（摘自 `docs/tool-catalog.md`——该文件由 `pnpm run gen-tool-catalog` 自动生成，
且有完整性守卫：新增 `packages/*/tool-*` 未进生成器清单即 CI 失败）

`bash` / `pwsh` / 持久 PTY 双份、`read` / `write` / `edit` / `read_image`、`str_replace_editor`、
`glob` / `grep`（内置 ripgrep 二进制，不依赖宿主）、`terminal_*`×6、`lsp`、`web_search` / `web_fetch`、
`skill`、`subagent` / `subagent_fork`、`send_message` / `interrupt_agent` / `list_agents` / `report`、
`job_kill|list|output`、`todo_write`、`workflow` / `ralph`、`session_*`×5、`create_goal` / `update_goal` / `get_goal`、
`schedule_*`×3、`ask_user_question`、`exit_plan_mode`、`run_code`（Code Mode：写 TS 程序批量调工具）、
`cordis_*`（运行时自我改写）、Agent Teams×10（实验）。

---

## 2. SeekCLI 现状盘点

| 维度 | 数据 |
| --- | --- |
| 规模 | **约 4800 行 Rust**，17 个源文件 |
| 测试 | 87 个单测，全部纯逻辑（无网络 / 无 e2e） |
| CI | 1 个 workflow：fmt / check / clippy / nextest / tag 时 release |
| 工具 | **8 个**：`read_file` `write_file` `edit_file` `list_dir` `run_shell` `invoke_agent` `create_skill` `load_skill` |
| 文档 | `AGENT_ARCHITECTURE.md` + `TODOs.md`（19 阶段）+ README + CHANGELOG |

---

## 3. 逐层评估

各层的**目标补全设计**已拆进 [`docs/architecture/`](../architecture/README.md) 对应文档，此处只列结论。

### L0 基底层 —— 70%

**已具备**：双 wire 协议（`api/openai.rs` + `api/anthropic.rs`）、SSE 流式解析、
**streaming tool-call 分片重组**（很多同类实现在这里是错的）、reasoning 内容剥离。

**缺口**：

| # | 缺口 | 证据 | 影响 |
| --- | --- | --- | --- |
| L0-1 | 没有真正的 provider 抽象 | `main.rs:88` 无论选哪个 provider 都读 `DEEPSEEK_API_KEY`，base_url 硬编码在 provider 内 | 换任何非 DeepSeek 端点做不到 |
| L0-2 | 无重试 / 指数退避 | `call_api_with_params(...).await?` 直接冒泡 | 一次 429 或 5xx 打断整个长任务 |
| L0-3 | 无请求超时 / 流空闲超时 | 无 | 服务端挂起即永久卡死 |
| L0-4 | 无 token 计数 | 压缩阈值用**字节数**（`COMPRESSION_THRESHOLD_BYTES = 600_000`） | 中英混排时阈值漂移可达数倍 |

→ 补全设计：[L0-llm-substrate.md](../architecture/L0-llm-substrate.md)

### L1 引擎层 —— 80%（**最强的一层**）

**已具备**：Two-Stage ReAct（宏 / 微双触发）、System Reminders 死循环干预、Error Recovery 提示注入、
`MAX_ITER=25`、子代理深度限制、Ctrl-C 优雅中断。

`engine.rs` 中对「假工具调用」的根因分析（阶段十九：规划轮以 assistant 结尾导致轮次结构失范；
`strip_fake_tool_syntax` 兜底切掉 U+FF5C 伪标签）——这类**从真实故障回推到分布外输入形状**的分析，
质量不输 dsh 的 agent note。

**缺口**：

| # | 缺口 | 证据 | 影响 |
| --- | --- | --- | --- |
| L1-1 | 循环封闭，无扩展点 | `run_agent_loop` 单函数 440 行，无 pre-step / post-step 钩子 | 每加一条策略都要改主控流本体 |
| L1-2 | 无 turn / step 概念 | 只有 `for iter in 0..max_iter` | 无法表达「一个 turn 含多个 step」，也无法在 step 边界做检查点 |
| L1-3 | 无运行中上下文注入 | 无 `agent.inject()` 对位 | 外部事件（文件变更、后台任务完成）无法进入下一轮请求 |
| L1-4 | 中断即终止，无法续跑 | Ctrl-C 后 `final_content = "[Interrupted by user]"` | 长任务被打断只能从头再来 |
| L1-5 | 取消不向下传播 | `run_shell` 子进程不接收取消信号 | Ctrl-C 后 shell 命令仍在后台跑 |

→ 补全设计：[L1-engine.md](../architecture/L1-engine.md)

### L2 边界层 —— 45%

**已具备**：静态 schema 全量注入、read-concurrent / write-serial 的 Fork-Join 并发
（`is_parallel_readonly` 的保守判定与注释都正确）、畸形 JSON 显式 `[BAD ARGS]`、大输出 offload。

**缺口**（按影响排序）：

| # | 缺口 | 证据 | 影响 |
| --- | --- | --- | --- |
| L2-1 | **无 MCP** | 全仓 grep `mcp` 零命中 | **最大单点缺口**：第三方无法扩展，只能改 Rust 源码 |
| L2-2 | **无原生 `glob` / `grep`** | `read_file` 描述直接教模型用 `run_shell` 配 sed/grep/head/tail | 每次检索都要过审批策略、依赖宿主装了什么、输出格式不可控。**日常代码任务成功率差异最大的一处** |
| L2-3 | 派发是硬编码 `match` | `tools/mod.rs::ToolDispatcher::execute` | 无 pre/post 钩子、无 per-tool 超时、结果一律 `String` |
| L2-4 | 无结构化 `ToolResult` | 用 `[USER DENIED]` / `[PATH DENIED]` / `[BAD ARGS]` 字符串前缀约定 | 阶段八 8.3 主动推迟项，至今未兑现；调用方靠 `contains` 猜语义 |
| L2-5 | 无后台执行 / job 控制 | `run_shell` 只能前台阻塞 | 跑不了 `cargo build --release` 这类长命令而不阻塞对话 |
| L2-6 | 无持久 PTY | 无 | 跑不了 REPL、`ssh`、交互式安装 |
| L2-7 | 无 `ask_user_question` | 人在环只覆盖 shell 审批一处 | 模型无法主动提问，只能猜或空转 |
| L2-8 | 无 `todo_write` | 用 PLAN.md / TODO.md 文件约定替代 | 合理的轻量选择，但状态不可被 UI 渲染、不进日志 |

→ 补全设计：[L2-tools.md](../architecture/L2-tools.md)、[L5-composition.md](../architecture/L5-composition.md)（MCP）

### L3 安全层 —— 50%，且**边界不一致**

**已具备**：三态 allow / ask / deny（`approval.rs`，config 可扩展）、
write 路径工作区白名单（`path_security.rs`，词法归一化不跟随符号链接，取舍注释清楚）。

**缺口**：

| # | 缺口 | 证据 | 影响 |
| --- | --- | --- | --- |
| L3-1 | **写受限、shell 不受限** | `ensure_within_cwd` 只挂在 `fs.rs:28`（write_file）与 `fs.rs:59`（edit_file） | `sh -c "cat > ~/.ssh/authorized_keys"` 只要不匹配 deny 子串就通过；文件白名单在有自由 shell 时只是防手滑 |
| L3-2 | **没有任何模式真正限制写入** | `engine.rs:339` 的 Plan Mode 只注入 `plan_mode_rules()` | 见下方订正 |
| L3-3 | 审批是子串匹配 | `approval::matches_any` 大小写不敏感子串 | `rm -rf` 变形、`$(...)`、别名均可绕过 |
| L3-4 | 无审计日志 | 无 | 事后无法追溯模型做了什么 |
| L3-5 | 无进程沙箱 | 无 | 越狱面是整个 `sh`。dsh 有 `packages/sandbox/` 四件套 + `native/landlock-run` |

> L3-5 已在 `AGENT_ARCHITECTURE §8.2` 主动声明为设计取舍，本评估**认可该取舍**，
> 但 L3-1 / L3-2 属于**边界不自洽**，应当补齐。

> **L3-2 订正**（2026-08-22，阶段二十四实施时发现）：本条初稿写作
> 「Plan Mode 只是 prompt，不阻断写工具」，是对照 dsh 时的**误判**。
> dsh 的 plan mode 意为「批准前不改任何东西」；SeekCLI 的 Plan Mode 是阶段十五的
> **状态外部化**，其提示词明确要求模型用 `write_file` 维护 PLAN.md / TODO.md。
> 两者同名而不同义——把 Plan Mode 接上写入限制，会直接破坏它所命名的功能。
> 真实缺口是**没有任何模式能表达「只看不改」**：阶段二十四以独立的
> `Mode::ReadOnly`（`--read-only` / `/readonly`）补齐，Plan Mode 语义保持不变。

→ 补全设计：[L3-security.md](../architecture/L3-security.md)、[security-model.md](../architecture/security-model.md)

### L4 记忆层 —— 45%（**架构级欠账**）

**已具备**：阶梯降级压缩（保留 ToolCall 意图链的方向正确）、工具大输出卸载、
session JSON + `/load` 前缀匹配、cost 随 session 落盘、PLAN.md / TODO.md 外部化。

**缺口**：

| # | 缺口 | 证据 | 影响 |
| --- | --- | --- | --- |
| L4-1 | **没有 append-only 事件日志** | `Session { messages: Vec<Message> }` 直接序列化 | fork / resume / 回放 / 遥测**每一项都得单独造**；dsh 全部由事件流免费派生 |
| L4-2 | 无 checkpoint / fork / resume-at-point | 只有整段 `/load` | 长任务无法「回到第 7 步换个方向」 |
| L4-3 | 无会话检索 | 无 | dsh 有 SQLite FTS + 5 个 `session_*` 工具让模型自查历史 |
| L4-4 | 无标题生成 | `title: "New Chat"` 硬编码 | `/history` 列表基本没法用 |
| L4-5 | `list_sessions` 全量读盘 | `history.rs::list_sessions` 反序列化每个 session | 用久线性劣化 |
| L4-6 | offload 无生命周期管理 | 只写 `~/.seekcli/tmp/<hash>.txt` | 无清理、无内容寻址去重 |

→ 补全设计：[L4-memory.md](../architecture/L4-memory.md)

### L5 组合层 —— 40%

**已具备**：SubAgent 类型化模板 + 工具裁剪 + 深度 3、
Skill 走 `SKILL.md`（agentskills.io 兼容，选型正确）+ **proposal 审核流**（拒绝在线自演化污染，判断正确）、
`load_skill` 工具化、Task × Skill 正交组合。

**缺口**：

| # | 缺口 | 影响 |
| --- | --- | --- |
| L5-1 | 无 MCP（同 L2-1） | 生态封闭 |
| L5-2 | 子代理一次性 | 返回 summary 即结束。dsh 有 continuable 子代理 + `send_message` / `interrupt_agent` / `list_agents` / 子代理侧 `report` |
| L5-3 | 无插件 / profile 组合机制 | SeekCLI 一切在编译期定死；dsh 一切在 `cordis.yml` patch 层可换 |
| L5-4 | 无 hooks | dsh 有 `hooks-claude-code` / `hooks-codex` 协议桥，可复用别家生态的 hook 脚本 |
| L5-5 | 无 workflow 编排 | dsh 有 `workflow` / `ralph` |

→ 补全设计：[L5-composition.md](../architecture/L5-composition.md)

### L6 界面层 —— 35%

**已具备**：干净 REPL、Tab 补全、spinner、prompt 显示 model / thinking / plan / skill 状态、`/copy`。

**缺口**：

| # | 缺口 | 影响 |
| --- | --- | --- |
| L6-1 | **无通用 headless 模式** | 只有 `--bench` 与 `--run-task <name>` 两个特化入口；没有 `seekcli -p "prompt"`，没有 stdin 管道。**堵死了被脚本 / CI 调用的全部场景，成本极低价值极高** |
| L6-2 | 无结构化输出 | 无 `--output json` | 无法被上层程序消费 |
| L6-3 | 无编辑器 / IDE 集成通道 | dsh 有 ACP server + JSON-RPC SDK + Python SDK |
| L6-4 | 无 TUI / Web UI | dsh 有完整 host + client 双半 Web 应用（本项目**不追求**此项，仅记录差距） |
| L6-5 | **配置文件读写 CWD**（**缺陷级**） | `config.rs:63` 读 `Path::new("config.toml")`——当前工作目录。换个目录即换套配置，且会到处撒文件。同见 §4 产品形态表 |

→ 补全设计：[L6-interface.md](../architecture/L6-interface.md)

### L7 可观测层 —— 55%

**已具备**：CostTracker（装饰器式，含缓存命中率）、Trace span 树落盘 `~/.seekcli/traces`、
Benchmark Runner + testbed 隔离 + eval 断言。**有 eval 循环这件事本身已超过绝大多数同类项目。**

**缺口**：

| # | 缺口 | 证据 | 影响 |
| --- | --- | --- | --- |
| L7-1 | **eval 数据集只有 1 个文件** | `examples/benchmarks/basic.json`（3 个任务） | 有跑道没有车，回归检测力接近零 |
| L7-2 | **agent 循环本身零自动化测试** | 87 个单测全是纯逻辑；无 LLM 录制 / 回放 | 主控流任何重构都只能靠手测 |
| L7-3 | 无快照回归 | 无 | 提示词 / 流程变更无差异可看。dsh 有 record / refresh / replay 三态快照 |
| L7-4 | 覆盖率门禁形同虚设 | `build.yml` 装了 `cargo-llvm-cov` 却从未调用 | CI 里那一步是死代码 |
| L7-5 | 无 OTel 导出 | trace 只能人眼看 JSON | |

→ 补全设计：[L7-observability.md](../architecture/L7-observability.md)

### L8 循环层 —— 60%

**已具备**：`--run-task` + launchd plist 示例 + `~/.seekcli/tasks` 状态外部化 + digest 通知 + Task × Skill 组合。
心智模型（Harness = 单次调用内 / Loop = 调用之间）清晰且正确。

**缺口**：

| # | 缺口 | 影响 |
| --- | --- | --- |
| L8-1 | 任务硬编码在 `tasks::task_spec` | 加一个任务要改 Rust 并重编译 |
| L8-2 | 模型无法自己排程 | dsh 有 `schedule_create|list|delete` + `goal` 三工具 |

→ 补全设计：[L8-loop.md](../architecture/L8-loop.md)

---

## 4. Agent Project 层面（作为可交付产品 / 开源项目）

| 项 | 状态 | 说明 |
| --- | --- | --- |
| **配置文件位置** | ❌ **缺陷** | `config.rs:63` 读 `Path::new("config.toml")`——**当前工作目录**。在任何目录跑 SeekCLI，它就在那儿写一个 `config.toml` |
| 凭证管理 | ❌ | 只认 `DEEPSEEK_API_KEY` 环境变量，anthropic provider 复用同一个 key；无 `.env`、无 keychain |
| 分发 | ❌ | 无 crates.io / homebrew / 预编译二进制；release 只在 `ubuntu-latest` 单平台，且不上传产物（只发 changelog） |
| LICENSE | ❌ | `Cargo.toml` 声明 MIT，仓库根**没有 LICENSE 文件** |
| 贡献治理 | ❌ | 无 CONTRIBUTING / issue 模板 / PR 模板 |
| 自身 agent 指令 | ❌ | 仓库里没有 AGENTS.md ——`prompt.rs::workspace_rules` 专门读别人的，自己却没有 |
| 文档生成 | ❌ | 全手写。dsh 的 tool-catalog / module-graph / config-catalog / cordis-api / persistence-catalog 全部生成 + CI 新鲜度门禁 |
| 设计过程留痕 | ⚠️ | `TODOs.md` 19 阶段带 commit 号，**已相当好**；但无 proposed / rejected 分态的决策档案 |
| i18n | ❌ | 无（dsh 每篇文档 en/zh 配对 + 配对校验脚本）。本项目单人中文开发，**不追求**此项 |

---

## 5. 优先级路线

完整任务拆解见根目录 [`TODOs.md`](../../TODOs.md)（阶段二十起）。此处只给优先级判断依据。

### P0 —— 日常可用性（几十到几百行，收益立刻可感）

| 项 | 对应缺口 | 理由 |
| --- | --- | --- |
| `config.toml` 改 `~/.seekcli/` + 项目级覆盖 | 工程 | 当前行为是 bug 级的 |
| 原生 `glob` / `grep` 工具 | L2-2 | **单项性价比最高**。用 `ignore` + `grep-searcher`，不依赖宿主 rg |
| `seekcli -p` + stdin + `--output json` | L6-1 / L6-2 | 打开脚本 / CI 调用场景，成本极低 |
| LLM 调用重试 / 退避 / 超时 | L0-2 / L0-3 | 长任务被一个 429 打断太亏 |
| 补 LICENSE | 工程 | 声明与事实不符 |

### P1 —— 打开生态（决定这个项目有没有第二个用户）

| 项 | 对应缺口 |
| --- | --- |
| **MCP client** | L2-1 / L5-1。接上之后 web / browser / database / GitHub 一整片能力不用自己写 |
| 多平台 release 产物 + `cargo install` 可用 | 工程 |
| `run_shell` 纳入路径策略 + plan mode 强制化 + 审计日志 | L3-1 / L3-2 / L3-4 |
| **Session 改 append-only 事件日志** | L4-1 ~ L4-5。**唯一建议推倒重来的地方，越晚改越贵** |

### P2 —— Harness 深水区

| 项 | 对应缺口 |
| --- | --- |
| 工具执行管线中间件化（Tool trait + ToolResult + 超时） | L2-3 / L2-4 |
| 后台 job 与 `job_*` 控制 | L2-5 |
| 循环扩展点重构（LoopHook） | L1-1 ~ L1-3 |
| eval 套件扩到 20+ 场景 + LLM 录制回放 | L7-1 / L7-2 |
| CI 接 `cargo-llvm-cov` 覆盖率门禁 | L7-4 |

### P3 —— 可选增强（明确不进主线）

OS 沙箱集成、OTel 导出、持久 PTY、workflow 编排、Web UI、i18n。

---

## 6. 本次评估的方法学说明

- 对 dsh 的判断来自其仓库内**自述文档与生成产物**（`docs/architecture.md`、
  `docs/tool-catalog.md`、`packages/README.md`、`package.json` scripts、workflow 清单），
  未逐包阅读源码；结构性结论可靠，实现细节不做断言。
- 对 SeekCLI 的判断来自**全量源码通读** + `git` 历史 + 实际运行 `cargo test` / `cargo deny`。
- 评分是**相对 dsh 这个坐标系**的完成度，不代表绝对质量。
  SeekCLI 是单人 5000 行 Rust 项目，dsh 是团队 56 万行 TS 项目，
  **不追求对齐规模，只借它当「应有组件验收清单」**——这与 SeekCLI 此前用
  harness-engineering 图3 当清单是同一种用法，只是清单更全。
