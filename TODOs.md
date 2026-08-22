# SeekCLI 进化路线图（阶段二十起）

> **定位**：单纯的本地 CLI Agent，核心 = DeepSeek + Tools + Harness Agent 引擎。
> **心智模型与逐层设计**：[`docs/architecture/`](docs/architecture/README.md)
> **缺口从哪来**：[`docs/evaluation/2026-08-harness-gap-analysis.md`](docs/evaluation/2026-08-harness-gap-analysis.md)
> **阶段一 ~ 阶段十九**：已全部完成并归档至 [`docs/archive/TODOs-phase-01-19.md`](docs/archive/TODOs-phase-01-19.md)

---

## 当前基线

| 项 | 值 |
| --- | --- |
| 版本 | `0.1.0` @ 阶段十九完成 |
| 规模 | 约 4800 行 Rust，17 个源文件 |
| 测试 | 87 单测（全纯逻辑） |
| 工具面 | 8 个 |
| 完成度 | 整体约 40%（引擎内核 65% / 产品形态 25%）——口径见 [scoring-rubric](docs/evaluation/scoring-rubric.md) |

## 本轮规划的三条主线

1. **先补可用性**（阶段二十 ~ 二十三）：几十到几百行的改动，收益立刻可感。
2. **再修边界与地基**（阶段二十四 ~ 二十七）：安全边界自洽、会话数据模型重构、工具管线中间件化。
   **这是唯一一处允许推倒重来的窗口，越晚改越贵。**
3. **最后打开生态与产品化**（阶段二十八 ~ 三十三）：MCP、后台任务、循环扩展点、分发。

### 依赖关系

```
20 工程基线 ──┬─→ 21 glob/grep
              ├─→ 22 L0 韧性
              └─→ 23 headless ──→ 29 eval 扩容
                                    ↑
24 L3 边界一致性                     │
                                    │
25 L7 录制回放（重构前置）──┬─→ 26 L4 事件日志重构 ──→ 31 L1 循环重构
                            ├─→ 27 L2 管线中间件化 ──→ 28 MCP ──→ 30 后台 job
                            └─────────────────────────────────────→ 32 L8 声明式任务
```

**阶段二十五必须先于二十六 / 二十七 / 三十一。** 没有可回放的测试，
把 440 行主循环和会话数据模型推倒重来是在裸奔。

---

## 🔴 P0：日常可用性

### 阶段二十：工程基线修复

*目标：修掉「在任何目录跑就往那儿写文件」这类 bug 级问题，把文档结构落定。*
*来源：评估 §4 产品形态 + L6-5。*

- [x] **20.1 文档结构重组**
    - [x] `docs/architecture/` —— 总纲 + L0~L8 逐层设计 + 设计原则 + 安全边界。
    - [x] `docs/evaluation/` —— 评分口径 + dsh 对照评估。
    - [x] `docs/archive/` —— 阶段一 ~ 十九路线图归档。
    - [x] 根 `AGENT_ARCHITECTURE.md` 降级为指针，`README.md` 更新链接。
- [ ] **20.2 配置定位修复**（L6-5，**缺陷级**）
    - [ ] 主配置移到 `~/.seekcli/config.toml`；首次运行生成**带注释**的默认配置。
    - [ ] 支持 `./.seekcli.toml` 项目级覆盖（仅覆盖出现的字段）与 `SEEKCLI_CONFIG` 显式指定。
    - [ ] 检测到 CWD 遗留 `config.toml` 且用户级不存在 → 打印迁移提示，**不自动搬**。
    - [ ] 单测：三级配置合并优先级。
- [x] **20.3 补 LICENSE**
    - [x] 根目录补 MIT LICENSE 文件（`Cargo.toml` 已声明 MIT，此前声明与事实不符，README 链接亦断开）。
- [ ] **20.4 补仓库自身的 AGENTS.md**
    - [ ] `prompt.rs::workspace_rules` 专门读工作区 AGENTS.md，本仓库却没有。
    - [ ] 内容：2 空格缩进、禁 unwrap/expect、提交规范、先跑 `cargo deny`。
- [ ] **20.5 CI 覆盖率门禁**（L7-4）
    - [ ] `build.yml` 里 `cargo-llvm-cov` 已安装却从未调用，是死代码；补上实际执行。
    - [ ] `--fail-under-lines 60` 起步，目的是防止「新增模块零测试」，不追求高数字。

**验收**：在任意目录运行 SeekCLI 不再产生 `config.toml`；CI 输出覆盖率并在低于阈值时失败。

---

### 阶段二十一：L2 原生检索工具

*目标：把检索从「教模型用 run_shell 配 sed/grep」变成一等工具。*
*来源：L2-2 —— 评估认定为**单项性价比最高**的改动。*
*设计：[L2 §4.1](docs/architecture/L2-tools.md#41-原生检索工具l2-2最高性价比)*

- [ ] **21.1 `tools/search.rs`**
    - [ ] 依赖 `ignore`（复用 ripgrep 的 gitignore 引擎）+ `grep-searcher`。
    - [ ] `glob(pattern, path?)` → 文件路径列表，按修改时间倒序，上限 200。
    - [ ] `grep(pattern, path?, glob?)` → `path:line: content`，上限 200。
    - [ ] **默认尊重 `.gitignore`**，避免把 `target/` 灌进上下文。
    - [ ] 超上限显式告知「还有 N 条未显示，请缩小范围」——不静默截断。
- [ ] **21.2 接入**
    - [ ] 注册进 `system_tools()`；加入 `is_parallel_readonly` 白名单。
    - [ ] 加进 `explore` SubAgent 模板的 `allowed_tools`。
    - [ ] 改 `read_file` description，删掉「用 run_shell 配 sed/grep/head/tail」的引导。
- [ ] **21.3 单测**：gitignore 生效 / 上限截断提示 / 空结果 / 无效正则报错。

**验收**：`grep("fn main", ".")` 直接返回结果，全程不触发审批提示；
在有 `target/` 的仓库里 `glob("**/*.rs")` 不返回构建产物。

---

### 阶段二十二：L0 韧性与 provider 配置化

*目标：长任务不再被一个 429 打断；能指向任意 OpenAI 兼容端点。*
*来源：L0-1 / L0-2 / L0-3。设计：[L0 §4.1-4.2](docs/architecture/L0-llm-substrate.md#4-目标设计)*

- [ ] **22.1 provider 配置化**（L0-1）
    - [ ] `config.toml` 改为 `[[provider]]` 数组 + `[brain] active`。
    - [ ] `api_key` 支持 `env:VAR` / `file:PATH` 前缀（为 keychain 留位置，不引入新依赖）。
    - [ ] `api::build_provider(&ProviderConfig)` 取代 `App::new` 里的硬编码分支。
    - [ ] **仍不做运行时多模型路由**——见 [design-principles](docs/architecture/design-principles.md)。
- [ ] **22.2 韧性装饰器**（L0-2 / L0-3）
    - [ ] `api/resilience.rs::Resilient<P>` 包装任意 `LlmProvider`，与 CostTracker 同一手法。
    - [ ] 指数退避 + jitter；429 优先尊重 `Retry-After`；4xx（非 429）立即失败。
    - [ ] `request_timeout`（默认 120s）+ `stream_idle_timeout`（默认 60s）。
    - [ ] **流中途断开且已产出 tool_calls 时不重试**——重试会重复副作用，交给 L1 Error Recovery。
- [ ] **22.3 单测**：退避序列 / 各错误码的重试判据 / 超时触发。

**验收**：断网或打 429 时 agent 循环不中断，日志显示退避重试；
`config.toml` 指向本地 vLLM 可正常对话；服务端不返回数据时按超时报错而非永久挂起。

---

### 阶段二十三：L6 headless 通用化

*目标：打开被脚本 / CI / 其它程序调用的全部场景。*
*来源：L6-1 / L6-2 —— 成本极低价值极高。设计：[L6 §4.1](docs/architecture/L6-interface.md#41-通用-headlessl6-1--l6-2)*

- [ ] **23.1 `-p` 一次性执行**
    - [ ] `seekcli -p "<prompt>"` 执行后打 stdout 并退出。
    - [ ] stdin 非 tty 时读入作为附加上下文（`cat x.md | seekcli -p "总结"`）。
    - [ ] `--max-iter` / `--read-only` / `--cwd` 参数。
- [ ] **23.2 结构化输出**
    - [ ] `--output json`：`session_id` / `final` / `iterations` / `usage` / `cost_cny` / `tools` / `status`。
    - [ ] **日志一律走 stderr**，stdout 只放结果，保证 `jq` 可直接解析。
- [ ] **23.3 非交互降级**
    - [ ] headless 下审批 `Ask` → `Deny`（除非 `--yes`）；`ask_user_question` → 非交互拒绝。
    - [ ] **绝不静默挂起**——无 TTY 时任何等待输入的路径都必须立即返回。
- [ ] **23.4 退出码语义**：0 完成 / 1 运行时错误 / 2 未收敛 / 3 被策略拒绝。
- [ ] **23.5 统一路径**：`--bench` 与 `--run-task` 内部改走同一条 headless 实现。

**验收**：`seekcli -p "统计仓库有多少个 .rs 文件"` 在 CI 环境正常返回并退出 0；
`--output json` 输出可被 `jq` 直接解析。

---

## 🟠 P1：边界与地基

### 阶段二十四：L3 安全边界一致性

*目标：消除「声明的边界与实际执行的边界不一致」——这比边界画得小危险得多。*
*来源：L3-1 / L3-2 / L3-3 / L3-4。设计：[L3 §4](docs/architecture/L3-security.md#4-目标设计)*

- [ ] **24.1 统一策略门**
    - [ ] `PolicyGate::check(tool, args) -> Verdict`，所有工具（含未来的 MCP 工具）走同一个门。
    - [ ] **模式门**：`Mode::{Normal, Plan, ReadOnly}`。Plan / ReadOnly 下 `write_file` /
          `edit_file` / `create_skill` 直接 `Deny`，`run_shell` 降级为只读白名单。
          —— 让 Plan Mode 从「提示模型别写」变成「写不了」（L3-2）。
    - [ ] **路径门**：`run_shell` 新增轻量写意图 + 路径提取（`>` `>>` `tee` `cp` `mv` `rm` `install`
          且路径在工作区外 → `Ask`）（L3-1）。**不做完整 shell AST 解析。**
- [ ] **24.2 命令解析升级**（L3-3）
    - [ ] `shlex` 分词后按 token 匹配，替代裸子串。
    - [ ] 识别 `;` / `&&` / `|` / `$(...)` 分隔的子命令，逐条判定取最严结果。
- [ ] **24.3 审计日志**（L3-4）
    - [ ] `~/.seekcli/audit.jsonl` 每次工具调用一行；记 `args_digest` 而非原文（避免写入密钥），
          `run_shell` 例外记完整命令。
- [ ] **24.4 单测**：模式门各组合 / 子命令取最严 / 路径提取 / 审计行格式。
- [ ] **24.5 同步更新** [`security-model.md`](docs/architecture/security-model.md) §5 的「已知不自洽」表。

**验收**：`/plan on` 后模型改文件被拒且文件未变；`run_shell("echo x > ~/.zshrc")` 触发审批；
`ls; rm -rf /tmp/foo` 按 `rm` 档位判定。

---

### 阶段二十五：L7 LLM 录制 / 回放（**重构前置条件**）

*目标：让 agent 主循环第一次具备自动化测试能力。*
*来源：L7-2。设计：[L7 §4.1](docs/architecture/L7-observability.md#41-llm-录制--回放l7-2优先级最高)*

> **这一阶段必须先于二十六 / 二十七 / 三十一。**
> 现在 87 个单测全是纯逻辑，`run_agent_loop` 零覆盖；
> 没有它，后面三次结构性重构都只能靠手测。

- [ ] **25.1 录制**：`SEEKCLI_RECORD=<dir>` 把每次 API 响应流按 `<seq>.jsonl` 落盘，
      另存 `request.json` 便于人读。
- [ ] **25.2 回放**：`SEEKCLI_REPLAY=<dir>` 从盘里按序喂回，零网络。
    - [ ] **校验请求形状**（消息条数 / 最后一条角色 / 工具集哈希），不匹配报错而非静默错位。
- [ ] **25.3 首批 fixture**
    - [ ] 「读不存在的文件 → 失败 → Two-Stage → 创建」完整轨迹（即阶段十九的复现场景）。
    - [ ] 多工具并发只读批次。
    - [ ] 触发压缩的长会话。
- [ ] **25.4 集成测试**：`#[tokio::test]` 跑通上述轨迹。
      故意改坏 `append_plan_with_bridge` 应导致测试失败。

**验收**：`cargo test` 中存在至少 3 条完整 agent 轨迹测试且全程零网络；
阶段十九的 bug 从此不会静默回归。

---

### 阶段二十六：L4 会话事件日志重构（**架构级**）

*目标：把会话从「压缩后的状态快照」改为 append-only 事件流。*
*来源：L4-1 ~ L4-6 —— 评估中**唯一建议推倒重来**的地方。*
*设计：[L4 §4](docs/architecture/L4-memory.md#4-目标设计)*

- [ ] **26.1 事件模型**
    - [ ] `SessionEvent { seq, ts, payload }`；`EventPayload` 覆盖 UserMessage / AssistantMessage /
          ToolResult / SystemPrompt / Compaction / SkillActivated / ModeChanged / Interrupted / Usage。
    - [ ] 会话目录化：`~/.seekcli/sessions/<id>/{meta.json, events.jsonl, blobs/}`。
    - [ ] `derive_messages(&[SessionEvent]) -> Vec<Message>` 成为**唯一**上下文来源。
- [ ] **26.2 压缩改造**
    - [ ] 压缩产出 `Compaction { replaced, summary }` **事件**，不再原地改写 messages。
    - [ ] **原始事件保留在文件里**——同时解决信息丢失与不可回放（兑现阶段十 10.3 推迟项）。
    - [ ] 阈值从字节改 token，接入 `TokenCounter`（L0-4）。
    - [ ] 删除现有幂等 marker 逻辑——事件流天然幂等。
- [ ] **26.3 派生能力**
    - [ ] `/resume <id>`、`/fork <id> [seq]`、`/search <kw>`（直接 grep events.jsonl，**不引入 SQLite**）。
    - [ ] `/history` 只读 `meta.json`，O(1) per session（L4-5）。
    - [ ] 标题生成：首条 UserMessage 后一次廉价补全写入 `meta.json`（L4-4）。
    - [ ] 中断写入 `Interrupted` 事件，配合 `/resume` 实现可续跑（L1-4）。
- [ ] **26.4 offload 归属**：blob 移到 `sessions/<id>/blobs/<sha256>.txt`，
      启动清理 30 天未访问，内容寻址天然去重（L4-6）。
- [ ] **26.5 迁移**：旧 `sessions/*.json` 一次性转目录格式，原文件 `.json.bak`；
      **可逆、失败回滚、不静默覆盖**（同阶段十二 `/skill migrate` 手法）。
- [ ] **26.6 测试**：投影正确性 / 压缩事件回放 / fork 边界 / 迁移幂等 + 阶段二十五的回放测试全过。

**验收**：长会话压缩后 `events.jsonl` 仍能读到原始内容；`/resume` 正确续接；
`/fork <id> 12` 的上下文等于前 12 个事件的投影；1000 会话时 `/history` < 100ms。

---

### 阶段二十七：L2 工具执行管线中间件化

*目标：把硬编码 `match` 换成注册表 + 中间件链，兑现阶段八 8.3 的推迟项。*
*来源：L2-3 / L2-4。设计：[L2 §4.2](docs/architecture/L2-tools.md#42-执行管线中间件化l2-3--l2-4)*

- [ ] **27.1 `ToolImpl` trait**：`name` / `schema` / `execute(args, cx) -> ToolResult`。
      现有 8 个工具逐个迁移。
- [ ] **27.2 结构化 `ToolResult`**
    - [ ] `ToolKind { Ok, Denied, Failed, Offloaded, BadArgs }`。
    - [ ] 正式取代 `[USER DENIED]` / `[PATH DENIED]` / `[BAD ARGS]` 的字符串前缀**判定**；
          前缀作为**给模型看的呈现层**保留，程序内部不再靠 `contains` 猜语义。
- [ ] **27.3 中间件链**：`approval → path policy → mode gate → timeout → execute → offload → recovery`。
    - [ ] per-tool timeout 默认 120s，`run_shell` 可覆盖至 600s；超时返回 `Failed` 并提示可用后台模式。
- [ ] **27.4 测试**：链顺序 / 各 `ToolKind` 分支 / 超时。

**验收**：新增一个工具只需实现 `ToolImpl` 并注册，不改 dispatcher；
所有既有行为经阶段二十五回放测试验证无变化。

---

### 阶段二十八：L5 MCP 客户端（**打开生态**）

*目标：让第三方能给 SeekCLI 加能力，而不必改 Rust 源码。*
*来源：L2-1 / L5-1 —— 评估认定的**最大单点缺口**。*
*设计：[L5 §4.1](docs/architecture/L5-composition.md#41-mcp-客户端l5-1)*

- [ ] **28.1 协议实现**
    - [ ] **只做 stdio transport**；手写 JSON-RPC（约 300 行，零新依赖树），不引入完整 SDK。
    - [ ] `initialize` → `tools/list` → 注册进工具注册表。
- [ ] **28.2 配置**：`config.toml` 的 `[[mcp]]` 数组（name / command / args / env / enabled）。
- [ ] **28.3 健壮性**
    - [ ] **单个 server 失败不影响启动**：警告 + 跳过（外部进程不可靠是常态）。
    - [ ] 启动超时上限（默认 10s/server），避免拖慢 REPL。
    - [ ] 进程随 REPL 退出而回收。
- [ ] **28.4 安全与命名**
    - [ ] 命名空间 `mcp__<server>__<tool>`。
    - [ ] MCP 工具**默认按非只读处理**（不进并发白名单），除非 server 明确声明只读。
    - [ ] 一律过阶段二十四的 `PolicyGate`；Plan / ReadOnly 模式下全拒。
- [ ] **28.5 `/tools` 命令**：列出当前生效工具及其来源（内置 / skill / MCP）。

**验收**：配置 filesystem server 后 `/tools` 出现 `mcp__filesystem__*` 且可调用；
故意写错某个 server 的 command，REPL 仍正常启动并只打印一条警告。

---

## 🟡 P2：产品化与深水区

### 阶段二十九：L7 评估闭环强化

*目标：让「引擎变好还是变坏」有数可依。*
*来源：L7-1。设计：[L7 §4.2](docs/architecture/L7-observability.md#42-eval-套件扩容l7-1)*

- [ ] **29.1 eval 套件扩到 20+ 任务**：`fs.json`(6) / `shell.json`(4) / `multistep.json`(5) /
      `recovery.json`(4) / `safety.json`(4)。
- [ ] **29.2 反向断言**：`safety.json` 期望模型**没有**做成某事，
      需在 Fail-to-Pass 范式上支持 `expect_fail: true`。
- [ ] **29.3 Cost / Trace 小改**
    - [ ] 费率从硬编码移进 `config.toml`，明确标注「估算」。
    - [ ] trace span 增加 `tool_calls` 计数与 `verdict`，
          让「模型声称完成但该轮 tool_calls=0」一眼可见（阶段十九靠人肉发现）。

**验收**：`--bench` 跑通全部 5 个套件并给出分套件成功率与成本。

---

### 阶段三十：L2 后台任务与 job 控制

*目标：长命令与子代理不再阻塞对话。*
*来源：L2-5 / L5-2。设计：[L2 §4.3](docs/architecture/L2-tools.md#43-后台任务l2-5)*

- [ ] **30.1 `JobRegistry`**：输出写 `~/.seekcli/jobs/<id>.log`，随 REPL 退出清理，**不做守护进程**。
- [ ] **30.2 工具**：`run_shell(background)` / `job_list` / `job_output(tail)` / `job_kill`。
- [ ] **30.3 完成通知**：经 L1 `inject()` 在下一轮告知模型，**不打断当前 step**。
- [ ] **30.4 后台子代理**：`invoke_agent(background)` 复用同一 registry；
      子代理事件写独立 session，主轴只记 `SubagentDelegated { session_id, summary }`。

**验收**：`run_shell("sleep 300", background=true)` 立即返回，`job_output` 能读增量输出。

---

### 阶段三十一：L1 循环拆解与扩展点

*目标：把 440 行主循环拆成可组合、可单测的阶段。*
*来源：L1-1 / L1-2 / L1-3 / L1-5。设计：[L1 §4](docs/architecture/L1-engine.md#4-目标设计)*

- [ ] **31.1 阶段函数拆解**：`prepare_step` / `request` / `dispatch_tools` / `observe`，
      主循环只做编排。**纯重构，行为零变化**，靠阶段二十五的回放测试守住。
- [ ] **31.2 `LoopHook` trait**：`pre_step` / `post_step` + `Control{Proceed,SkipStep,StopTurn}`；
      compressor / reminders / tracer 改造成 hook。
    - [ ] **不做事件总线 / waterfall / 动态注册**——静态 `Vec<Box<dyn LoopHook>>` 足够。
- [ ] **31.3 取消传播**（L1-5）：`CancelToken` 传到 `run_shell`，
      `tokio::select!` 竞争 + `child.start_kill()`。
- [ ] **31.4 `inject()`**（L1-3）：外部事件进入下一次 `prepare_step`。

**验收**：Ctrl-C 时正在跑的 `sleep 60` 子进程 1s 内消失（`ps` 可验证）；
新增循环策略只需实现 `LoopHook`；回放测试与 trace 树结构不变。

---

### 阶段三十二：L8 任务声明式化

*目标：加一个定时任务不再需要改 Rust。*
*来源：L8-1。设计：[L8 §4.1](docs/architecture/L8-loop.md#41-任务声明式化l8-1)*

- [ ] **32.1 `TASK.md` 格式**：与 Skill **完全同构**，复用既有 frontmatter parser
      （name / description / skill / interval_hint + Markdown body 作为 prompt）。
- [ ] **32.2 `task_spec` 硬编码 match 改为读 `~/.seekcli/tasks/<name>/TASK.md`**；
      找不到时报错并列出可用任务。
- [ ] **32.3 内置 reminders 首次运行自动写出默认 `TASK.md`**，用户可直接改。
- [ ] **32.4 `seekcli task install <name>`**：按 `interval_hint` 生成 plist 到 stdout，
      **不提供自动安装器**——往用户 LaunchAgents 塞东西应当是显式动作。

**验收**：新增任务只写一个 `TASK.md` 即可运行；现有 reminders 行为不变。

---

### 阶段三十三：分发与发布

*目标：让第二个用户装得上。*
*来源：评估 §4 产品形态。*

- [ ] **33.1 多平台 release**：`build.yml` 矩阵扩到 macOS(arm64/x86_64) + Linux(x86_64/arm64)，
      tag 时上传二进制产物（当前只发 changelog，不传产物）。
- [ ] **33.2 `cargo install seekcli` 可用**：补 `description` / `repository` / `keywords` /
      `readme` 等 crates.io 必需元数据。
- [ ] **33.3 README 重写**：安装 / 快速上手 / 配置 / MCP 接入，面向新用户而非作者。
- [ ] **33.4 CONTRIBUTING + issue 模板**。

**验收**：在一台干净机器上 `cargo install seekcli` 后可直接使用。

---

## ⚪ P3：明确不排期

以下项**有意不做**，理由见 [design-principles §2](docs/architecture/design-principles.md#2-明确排除的复杂度)
与 [security-model §6](docs/architecture/security-model.md#6-未来增强方向不在主线)。
若将来重估，需先修改对应设计文档再开阶段。

| 项 | 理由 |
| --- | --- |
| OS 沙箱（Landlock / Seatbelt / bwrap） | 威胁模型不同；需要隔离请在容器内运行 |
| 持久 PTY / `terminal_*` 工具族 | `run_shell` + 后台 job 覆盖 90% 场景 |
| Code Mode（`run_code`） | 收益在超大工具面时才显现 |
| workflow / DAG 编排 | 是另一个产品 |
| 跨会话语义记忆 | 与 CLI 即时性目标背离 |
| 插件框架 / profile / bundle | 扩展需求由 MCP 承担 |
| TUI / Web UI / ACP server | 与本地 CLI 定位冲突；`--output json` 覆盖多数集成需求 |
| OTel 导出 | 单机无 collector；JSON span + `jq` 已够 |
| 文档 i18n | 单人中文开发 |

---

## 附：已完成阶段速查

阶段一 ~ 十九详见 [归档路线图](docs/archive/TODOs-phase-01-19.md)。

| 阶段 | 主题 | 层 |
| --- | --- | --- |
| 一 ~ 四 | 多模态 / 文档解析 / 联网搜索（**已于阶段七剥离**） | — |
| 五 ~ 六 | Harness 核心引擎 + schema 注册 + 迭代上限 | L1/L2 |
| 七 | 外围资产剥离，回归纯 CLI Agent | — |
| 八 | 危险命令审批 + 路径白名单 | L3 |
| 九 | SubAgent 类型化 + Skill proposal 审核 | L5 |
| 十 | 上下文压缩 + prompt cache 验证 | L4 |
| 十一 | REPL 瘦身 + Ctrl-C + 补全 | L6 |
| 十二 | SKILL.md 格式标准化（agentskills.io 兼容） | L5 |
| 十三 | Two-Stage ReAct / Reminders / Recovery / Fork-Join / 动态 AGENTS.md | L1/L2 |
| 十四 | 阶梯降级压缩 + 工具大输出卸载 | L4 |
| 十五 | 状态外部化 + Plan Mode | L4 |
| 十六 | Human-in-loop 三态权限 | L3 |
| 十七 | Cost Tracker + Tracing + Benchmark Runner | L7 |
| 十八 | headless 定时任务 + digest 通知 | L8 |
| 十九 | Two-Stage 假工具调用根因修复 | L1 |
