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

### ✅ 阶段二十：工程基线修复

*目标：修掉「在任何目录跑就往那儿写文件」这类 bug 级问题，把文档结构落定。*
*来源：评估 §4 产品形态 + L6-5。*

- [x] **20.1 文档结构重组**
    - [x] `docs/architecture/` —— 总纲 + L0~L8 逐层设计 + 设计原则 + 安全边界。
    - [x] `docs/evaluation/` —— 评分口径 + dsh 对照评估。
    - [x] `docs/archive/` —— 阶段一 ~ 十九路线图归档。
    - [x] 根 `AGENT_ARCHITECTURE.md` 降级为指针，`README.md` 更新链接。
- [x] **20.2 配置定位修复**（L6-5，**缺陷级**）
    - [x] 主配置移到 `~/.seekcli/config.toml`；首次运行生成**带注释**的默认配置。
    - [x] 支持 `./.seekcli.toml` 项目级覆盖（仅覆盖出现的字段）与 `SEEKCLI_CONFIG` 显式指定。
          合并在 `toml::Value` 层做深合并，新增配置字段无需改动加载逻辑。
    - [x] 检测到 CWD 遗留 `config.toml` 且用户级不存在 → 打印迁移提示（含 cp 命令），**不自动搬**。
    - [x] 启动提示走 stderr，为阶段二十三的 `--output json` 预留干净的 stdout。
    - [x] 顺带删除阶段七遗留的死配置 `[sensor] vlm_model`，以及仓库里已无人读取的 `config.toml`。
    - [x] 8 个新单测：首次生成 / 逐键覆盖 / env 层最高 / 缺失 explicit 报错 /
          部分层保留默认 / 迁移提示触发与消失 / 坏 TOML 指名文件。
- [x] **20.3 补 LICENSE**
    - [x] 根目录补 MIT LICENSE 文件（`Cargo.toml` 已声明 MIT，此前声明与事实不符，README 链接亦断开）。
- [x] **20.4 补仓库自身的 AGENTS.md**
    - [x] `prompt.rs::workspace_rules` 专门读工作区 AGENTS.md，本仓库却没有。
    - [x] 内容：先读哪份设计文档、2 空格缩进、禁 unwrap/expect、架构约束、
          改 LLM 交互时的专项注意事项、提交前门禁、提交规范、目录速览。
    - [x] 4664 字节，在 `WORKSPACE_RULES_CAP = 8192` 之内（超限会被截断注入）。
- [x] **20.5 CI 覆盖率门禁**（L7-4）
    - [x] `build.yml` 里 `cargo-llvm-cov` 已安装却从未调用，是死代码；改为
          `cargo llvm-cov nextest --summary-only --fail-under-lines 45`，
          一条命令同时跑测试与测覆盖率，门禁不会与实际跑的测试脱节。
    - [x] 阈值按**实测**定为 45（原计划 60 是拍脑袋）：采纳时行覆盖率 46.29%，
          取略低于基线的值作为棘轮。已验证 45 通过、60 失败（退出码 1）。
    - [x] 实测顺带量化了 L7-2：`engine.rs` 行覆盖率仅 13.8%，
          `tools/shell.rs` / `tools/fs.rs` / `api/openai.rs` / `commands.rs` 为 0%
          —— 印证阶段二十五（录制回放）必须前置于三次结构性重构。

**验收**：在任意目录运行 SeekCLI 不再产生 `config.toml`；CI 输出覆盖率并在低于阈值时失败。

---

### ✅ 阶段二十一：L2 原生检索工具

*目标：把检索从「教模型用 run_shell 配 sed/grep」变成一等工具。*
*来源：L2-2 —— 评估认定为**单项性价比最高**的改动。*
*设计：[L2 §4.1](docs/architecture/L2-tools.md#41-原生检索工具l2-2最高性价比)*

- [x] **21.1 `tools/search.rs`**
    - [x] 依赖 `ignore` + `grep-searcher` / `grep-regex` / `grep-matcher`。
    - [x] `glob(pattern, path?)` → 文件路径列表，按修改时间倒序，上限 200。
    - [x] `grep(pattern, path?, glob?)` → `path:line: content`，上限 200。
    - [x] **默认尊重 `.gitignore`**，且用 `require_git(false)` 让非 git 目录同样生效；
          显式 `filter_entry` 掉 `.git`（否则 `hidden(false)` 会走进 object 文件）。
    - [x] 超上限显式告知「还有 N 条未显示」——不静默截断。
    - [x] 同步 IO 走 `spawn_blocking`，不阻塞并发批次依赖的运行时。
    - [x] 单行上限 300 字符，防止一行压缩 bundle 挤掉其余结果。
    - [x] 无效正则返回 `[BAD PATTERN]` 并提示「这是正则不是 glob」，而非「无匹配」——
          后者会让模型误判代码不存在。
- [x] **21.2 接入**
    - [x] 注册进 `system_tools()`；加入 `is_parallel_readonly` 白名单。
    - [x] 加进 `explore` 与 `general` SubAgent 模板的 `allowed_tools` 与提示。
    - [x] 改 `read_file` / `list_dir` description，删掉「用 run_shell 配 sed/grep/find」的引导。
    - [x] 系统提示新增 glob/grep 段落与「如何选择」条目。
- [x] **21.3 单测**（11 个）：gitignore 生效 / 目录限定 / 空结果措辞 / glob 过滤 /
      无效正则 / 路径不存在带恢复提示 / 截断计数 / UTF-8 边界 /
      **对真实仓库的冒烟测试**（构建产物与 .git 不得泄漏）。

**验收**：`grep("fn main", ".")` 直接返回结果，全程不触发审批提示；
在有 `target/` 的仓库里 `glob("**/*.rs")` 不返回构建产物。

---

### ✅ 阶段二十二：L0 韧性与 provider 配置化

*目标：长任务不再被一个 429 打断；能指向任意 OpenAI 兼容端点。*
*来源：L0-1 / L0-2 / L0-3。设计：[L0 §4.1-4.2](docs/architecture/L0-llm-substrate.md#4-目标设计)*

- [x] **22.1 provider 配置化**（L0-1）
    - [x] `config.toml` 新增 `[[provider]]` 端点表；`[brain] provider` 指向其中一项。
          **保持向后兼容**：写着 `provider = "openai"/"anthropic"` 的旧配置，
          在没有同名 `[[provider]]` 时合成内置 DeepSeek 端点，行为不变。
          （未采用设计稿里的 `[brain] active`——那会破坏 `/model flash|pro` 的既有语义。）
    - [x] `api_key` 只接受 `env:VAR` / `file:PATH`；**字面量 key 直接拒绝启动**——
          配置文件会被误提交，启动失败远比泄露凭证便宜。`file:` 支持 `~/` 展开，零新依赖。
    - [x] `api::build_provider(&Config)` 取代 `App::new` 里的硬编码分支；
          `DEEPSEEK_API_BASE` / `DEEPSEEK_ANTHROPIC_BASE` 环境变量覆盖能力保留。
    - [x] 未知 wire 在解析期报错（而非首次请求时）；未知 provider 名报错时列出可用名字。
    - [x] **仍不做运行时多模型路由**。
- [x] **22.2 韧性装饰器**（L0-2 / L0-3）
    - [x] `api/resilience.rs::Resilient` 包装 `Box<dyn LlmProvider>`，与 CostTracker 同一手法，
          `run_agent_loop` 零改动。
    - [x] 类型化 `LlmError{Status, Transport}` 取代原先的 `bail!("API Error {status}")` 字符串——
          否则装饰器无法判断该不该重试。
    - [x] 指数退避 + **确定性** jitter（随机 jitter 会让退避序列不可测试）；
          429/408/5xx/传输错误重试，其余 4xx 立即失败，**未分类错误也不重试**。
    - [x] `Retry-After` 只解析 delta-seconds 形式，且与退避共用 `max_delay` 上限——
          服务端要求等一小时不能把 agent 挂住。
    - [x] `request_timeout`（120s，仅到响应头）+ `stream_idle_timeout`（60s，两 chunk 之间）。
    - [x] **只重试返回流之前的那次调用**，一旦出字节就永不重试——判断「已产出 tool_calls」
          需缓冲整条流，收益不抵成本；中途失败交给 L1 Error Recovery。
    - [x] 重试时打印一行可见提示（降级不静默）。
- [x] **22.3 测试**（16 个）：重试判据分类 / 未分类错误不重试 / 退避指数增长与封顶 /
      `Retry-After` 覆盖且受限 / 头部解析 / **真正驱动装饰器**的四个用例
      （瞬时失败重试成功且计数=3、400 立即失败且计数=1、达上限停止、
      静默流触发 idle 超时而非永久挂起）。
    - [x] 真实 API 端到端验证：`--run-task reminders` 4 次调用跑通，cache hit 97%。

**验收**：断网或打 429 时 agent 循环不中断，日志显示退避重试；
`config.toml` 指向本地 vLLM 可正常对话；服务端不返回数据时按超时报错而非永久挂起。

---

### ✅ 阶段二十三：L6 headless 通用化

*目标：打开被脚本 / CI / 其它程序调用的全部场景。*
*来源：L6-1 / L6-2 —— 成本极低价值极高。设计：[L6 §4.1](docs/architecture/L6-interface.md#41-通用-headlessl6-1--l6-2)*

- [x] **23.1 `-p` 一次性执行**
    - [x] `seekcli -p "<prompt>"` 执行后打 stdout 并退出。
    - [x] stdin **仅在非 TTY 时**读入作为附加上下文（读交互式 stdin 会阻塞等 EOF，
          正是本阶段要消灭的挂起）。
    - [x] `--max-iter` / `--read-only` / `--yes` / `--cwd` 参数。
- [x] **23.2 结构化输出**
    - [x] `--output json`：`final` / `status` / `iterations` / `llm_calls` / `usage` / `cost_cny`。
          （`session_id` 待阶段二十六事件日志落地后补——当前 headless 不落会话。）
    - [x] 新增 `src/ui.rs` 做输出分流：进度、流式输出、审批提示、重试通知一律 stderr，
          结果只在 stdout 出现一次。REPL 里流式输出**就是**结果，故仍走 stdout——
          这是需要模式开关而非无条件重定向的原因。
    - [x] engine/shell/compressor/search/tasks 共 32 处 `println!` 改 `eprintln!`。
- [x] **23.3 非交互降级**
    - [x] headless 下审批 `Ask` → `Deny`（`--yes` 则 `AutoApprove`）；
          `approval::Interaction` 三态，锁中毒时回落到 `Prompt` 而非静默放行。
    - [x] `--read-only` 用 `tools::ExecMode` 真正拦截 write_file / edit_file /
          run_shell / create_skill，返回 `[MODE DENIED]` 给模型而非抛错中断。
          `run_shell` 整体拒绝而非猜测其副作用——一个 `>` 重定向能走过去的门禁不算门禁。
          阶段二十四会把它并入 `PolicyGate` 与 Plan Mode 一起处理。
    - [x] `ask_user_question` 工具本身在阶段二十七 27.4 落地（此时尚不存在）。
- [x] **23.4 退出码语义**：0 完成 / 1 运行时错误 / 2 未收敛 / 3 被中断。
      `run_agent_loop` 返回值结构化为 `LoopResult{text,messages,status,iterations}`，
      `LoopStatus` 区分 Completed / MaxIterations / Interrupted。
- [x] **23.5 统一路径**：`--bench` / `--run-task` 与 `-p` 共用 headless 标志与降级策略。

**真实端到端验证**（6 项全过）：
- `-p` 文本模式 stdout 只有结果、退出码 0
- `--output json` 可直接喂 `jq`
- `echo ... | seekcli -p` 读到管道内容
- `--read-only` 下模型无法创建文件（磁盘验证）
- headless 下 `sudo` 命令自动拒绝且**不挂起**
- `--max-iter 1` 未收敛 → `status: max_iterations` 且退出码 2

**验收**：`seekcli -p "统计仓库有多少个 .rs 文件"` 在 CI 环境正常返回并退出 0；
`--output json` 输出可被 `jq` 直接解析。

---

## 🟠 P1：边界与地基

### ✅ 阶段二十四：L3 安全边界一致性

*目标：消除「声明的边界与实际执行的边界不一致」——这比边界画得小危险得多。*
*来源：L3-1 / L3-2 / L3-3 / L3-4。设计：[L3 §4](docs/architecture/L3-security.md#4-目标设计)*

- [x] **24.1 统一策略门**
    - [x] `tools/policy.rs::check(tool, args) -> Verdict`，所有工具（含未来的 MCP 工具）
          走同一个门：模式门 → 路径门 → 命令门。
    - [x] **模式门**：`Mode::{Normal, ReadOnly}`。ReadOnly 下 `write_file` / `edit_file` /
          `create_skill` 直接 `Deny`；`run_shell` 降级为「每个子命令 argv[0] 在只读名单内
          且全句无重定向」，`git push` / `cargo build` 这类只读程序的写子命令也排除。
    - [x] ⚠️ **计划修正**：原计划让 Plan Mode 也进模式门，实施时发现是**误判**——
          本项目的 Plan Mode 意为「状态外部化」，提示词明确要求模型写 PLAN.md / TODO.md，
          限制写入会破坏它所命名的功能。改为独立的 `--read-only` / `/readonly`，
          Plan Mode 语义不变。已同步订正评估 L3-2、L3 架构文档与 security-model §5，
          并留了一个回归单测 `plan_mode_is_not_a_write_restriction`。
    - [x] **路径门**：`policy::escaping_write_targets` 检测重定向或写动词
          （tee/cp/mv/rm/install/touch/mkdir/dd/chmod/chown/ln/…）指向绝对或 `~` 路径 → `Ask`（L3-1）。
          **不做完整 shell AST 解析。**
- [x] **24.2 命令解析升级**（L3-3）
    - [x] `policy::split_subcommands` 按 `;` `&&` `||` `|` `&` 拆分，并把 `$(...)` 与反引号
          的内容当作独立命令递归处理。逐条判定取最严——`ls; rm -rf /` 不再由 `ls` 决定。
    - [x] `shlex` 分词取 argv[0] 判定只读名单；危险模式检测仍走既有 token 边界匹配
          （重写检测器风险高于收益，且已有 11 个单测覆盖）。
- [x] **24.3 审计日志**（L3-4）
    - [x] `tools/audit.rs` → `~/.seekcli/audit.jsonl`，每次工具调用一行。
    - [x] 参数记 FNV-1a 摘要而非原文——工具参数常含文件内容、偶尔含密钥，
          **为安全而写的日志不能自己变成泄露源**；`run_shell` 例外记完整命令
          （反正执行前已打印到终端，隐藏它只会丢掉价值而不减少暴露）。
    - [x] 写盘失败只告警不中断，但不静默。
- [x] **24.4 单测**（8 个）：子命令拆分各分隔符 / 无害前缀不再决定判定 / 取最严 /
      只读 shell 名单（含 git push、cargo build、重定向、未知程序、单条污染全句）/
      越界写检测（工作区内不误报、读取不误报）/ 只读模式各工具 / 普通模式路径与命令门 /
      **Plan Mode 不是写入限制**的回归守卫。
- [x] **24.5 同步更新** security-model §5、L3 架构文档、评估 L3-2 订正。

**真实端到端验证**（4 项全过）：
- headless 下 `echo pwned > <工作区外路径>` 被拒，磁盘验证文件未创建
- `ls && sudo whoami` 被 sudo 决定判定（此前 `ls` 前缀会让整行看起来无害）
- `--read-only` 下 `ls` 可执行、`rm victim.txt` 被 `[MODE DENIED]`，文件仍在
- `~/.seekcli/audit.jsonl` 记录了 executed 与 denied 两类条目

**验收**：`/plan on` 后模型改文件被拒且文件未变；`run_shell("echo x > ~/.zshrc")` 触发审批；
`ls; rm -rf /tmp/foo` 按 `rm` 档位判定。

---

### ✅ 阶段二十五：L7 LLM 录制 / 回放（**重构前置条件**）

*目标：让 agent 主循环第一次具备自动化测试能力。*
*来源：L7-2。设计：[L7 §4.1](docs/architecture/L7-observability.md#41-llm-录制--回放l7-2优先级最高)*

> **这一阶段必须先于二十六 / 二十七 / 三十一。**
> 现在 87 个单测全是纯逻辑，`run_agent_loop` 零覆盖；
> 没有它，后面三次结构性重构都只能靠手测。

- [x] **25.1 录制**：`SEEKCLI_RECORD=<dir>` 把每次响应流写 `<seq>.jsonl`，
      请求形状写 `<seq>.request.json`。装饰器放在重试层**内侧**——
      录下的是真正到达循环的那条流，不是路上失败的尝试。
- [x] **25.2 回放**：`SEEKCLI_REPLAY=<dir>` 按序喂回，零网络。
    - [x] 在**解析凭证之前**短路：回放必须能无 API key 运行，否则它使能的测试进不了 CI。
    - [x] **校验请求形状**（消息条数 / 最后一条角色 / 工具集），不匹配硬失败。
          刻意只查结构不查逐字内容：改措辞不该让整套 fixture 失效，换一段对话必须失败。
    - [x] 回放耗尽时报「轨迹与 fixture 分叉」并给出处理建议，而非静默错位。
- [x] **25.3 首批 fixture**（`tests/fixtures/`，76K，全部录自真实 API）
    - [x] `two-stage-recovery`：读不存在的文件 → 失败 → 规划轮 → 创建（阶段十九复现场景）。
    - [x] `parallel-readonly`：glob + grep 同轮并发，走 Fork-Join 路径。
    - [x] `readonly-denial`：只读模式下策略门拒绝写。
    - [ ] ~~触发压缩的长会话~~ —— 录一条需要几十万 token，成本不划算；
          压缩已有 3 个纯逻辑单测，待阶段二十六改成事件流后再评估。
- [x] **25.4 集成测试**（4 个，`src/loop_tests.rs`）：首次真正驱动 `run_agent_loop`。
    - [x] 断言**副作用**优先于文本——阶段十九 bug 的特征正是模型「声称」做完了。
    - [x] 已验证：故意去掉 `append_plan_with_bridge` 的桥接消息 → 该测试失败。
    - [x] 踩到三处进程级全局状态（cwd / `policy::MODE` / `approval::INTERACTION`）
          与并行测试的冲突，用 `crate::testsync` 单锁串行化。

**顺带修掉一个被新测试挖出来的真实缺陷**：`planning_phase` 此前签名是 `&self`，
`Usage` 走 `_ => {}` 被丢弃——**Two-Stage 规划轮消耗的 token 从未进账单**。
而失败的轮次会触发规划轮，所以运行越糟糕，账单少算得越多。改为 `&mut self` 并记账。

**覆盖率**：`engine.rs` 13.8% → **65.0%**，总体 46.3% → **65.6%**。
CI 门禁棘轮从 45 上调到 60。

**验收**：`cargo test` 中存在至少 3 条完整 agent 轨迹测试且全程零网络；
阶段十九的 bug 从此不会静默回归。

---

### ✅ 阶段二十六：L4 会话事件日志重构（**架构级**）

*目标：把会话从「压缩后的状态快照」改为 append-only 事件流。*
*来源：L4-1 ~ L4-6 —— 评估中**唯一建议推倒重来**的地方。*
*设计：[L4 §4](docs/architecture/L4-memory.md#4-目标设计)*

- [x] **26.1 事件模型**
    - [x] `SessionEvent { seq, ts, payload }`；`EventPayload` 覆盖 UserMessage / AssistantMessage /
          ToolResult / SystemPrompt / Compaction / SkillActivated / ModeChanged / Interrupted / Usage。
    - [x] 会话目录化：`~/.seekcli/sessions/<id>/{meta.json, events.jsonl, blobs/}`。
    - [x] `derive_messages(&[SessionEvent]) -> Vec<Message>` 成为**唯一**上下文来源。
    - [x] `engine::log_push` 让「追加消息」与「追加事件」成为同一个动作——
          否则某处只写其一时，「模型可见即已记录」会悄悄失效且无人报错。
    - [x] `LoopResult` **不再返回工作集**：多给调用方一份对话副本，正是旧快照格式
          丢失压缩历史的原因。
- [x] **26.2 压缩改造**
    - [x] 新增 `maybe_compact_session`：在**轮次边界**产出 `Compaction` 事件，摘要进日志并跨轮沿用。
          轮内 `maybe_compress` 保留为单轮膨胀的安全网（掩码/截断是幂等内容变换，不丢数据）。
    - [x] `derive_messages_indexed` 提供消息区间 → 事件序号映射；没有它，唯一能表达的压缩
          就只有「全部替换」，会连刻意保留的近期尾部一起丢掉。
    - [x] **原始事件保留在文件里**——同时解决信息丢失与不可回放（兑现阶段十 10.3 推迟项）。
    - [x] 阈值从字节改 token（L0-4）：新增 `api/tokens.rs::{TokenCounter, Heuristic}`，
          600_000 字节 → 150_000 token。旧口径下同一段对话用中文写，会在真实预算
          约三分之一处就触发压缩。计入工具调用的 name/arguments 与每条消息的封装开销。
    - [x] ~~删除幂等 marker~~ —— 保留：轮内掩码仍作用于工作集，marker 仍是它的幂等保证。
- [x] **26.3 派生能力**
    - [x] `/resume <id>`（`/load` 保留为别名）、`/fork <id> [n]`（L4-2）。
          歧义前缀直接报错而不是静默挑一个。
    - [x] `/search <kw>`：扫描 events.jsonl，**不引入 SQLite**（L4-3）。摘要显示消息正文而非 JSON 记录。
    - [x] `/history` 只读 `meta.json`，O(1) per session（L4-5）。
    - [x] 标题取首条提示词首行（L4-4）。改用 LLM 生成需要一次额外调用，先看首行够不够用。
    - [x] 中断写入 `Interrupted` 事件，配合 `/resume` 实现可续跑（L1-4）。
- [x] **26.4 offload 归属**：blob 移到 `sessions/<id>/blobs/<sha256>.txt`，
      启动清理 30 天未访问，内容寻址天然去重（L4-6）。
- [x] **26.5 迁移**：旧 `sessions/*.json` 一次性转目录格式，原文件 `.json.bak`；
      **可逆、失败回滚、不静默覆盖**（同阶段十二 `/skill migrate` 手法）。
- [x] **26.6 测试**（18 个新增）：投影顺序 / 记账事件不可见 / 压缩投影替换但磁盘保留 /
      fork 截断与谱系 / fork 越界钳制 / jsonl 往返 / 单行损坏不丢会话 /
      存取往返 / 列表只读 meta 且倒序 / 歧义前缀拒绝 / 迁移可逆 / 迁移幂等 /
      搜索大小写不敏感 / 搜索摘要不泄漏 JSON / 消息转换覆盖所有角色 /
      **日志不变量**（模型看到的一切都能从日志重建）/ tool 消息带 call_id 往返。

**顺带修掉一个潜伏已久的真实 bug**：`Message` 的 `#[serde(untagged)]` 把 `Simple`
声明在前，而 serde 默认忽略未知字段，于是 `{role:"tool", content, tool_call_id}`
会匹配到 `Simple` 并**静默丢掉 `tool_call_id`**——任何含工具结果的会话 `/load`
回来都是残缺历史。迁移把它暴露了（6 条消息只迁出 4 条）。调换变体顺序并加了
往返回归单测。

**顺带调整**：`App::new` 把会话存储初始化提到 provider 之前——存储不依赖凭证，
先迁移意味着用户之后补上 key 时不会发现历史还卡在旧格式。

**真实端到端验证**：69 个历史会话全部迁移且保真（6 事件 ↔ 6 消息，原文件留 `.json.bak`）；
`/history` `/search` `/fork` 正常；**新进程里 `/resume` 后模型正确答出上一会话才知道的数字**。

**覆盖率** 65.6% → 67.9%（`session.rs` 96.7%，`history.rs` 87.0%）。

**验收**：长会话压缩后 `events.jsonl` 仍能读到原始内容；`/resume` 正确续接；
`/fork <id> 12` 的上下文等于前 12 个事件的投影；1000 会话时 `/history` < 100ms。

---

### ✅ 阶段二十七：L2 工具执行管线中间件化

*目标：把硬编码 `match` 换成注册表 + 中间件链，兑现阶段八 8.3 的推迟项。*
*来源：L2-3 / L2-4。设计：[L2 §4.2](docs/architecture/L2-tools.md#42-执行管线中间件化l2-3--l2-4)*

- [x] **27.2 结构化 `ToolResult`**
    - [x] `ToolKind { Ok, Denied, Failed, BadArgs, TimedOut }`，兑现阶段八 8.3 的推迟项。
    - [x] 前缀保留为**给模型看的呈现层**（系统提示已训练它识别），但程序内部不再
          `contains` 猜语义——`kind` 承载含义，前缀由 kind 渲染出来而不是被解析回去。
    - [x] **`Denied` 不算失败**：拒绝是决定不是故障。当成失败会让模型重新规划绕开策略，
          而那正是策略存在的意义；Error Recovery 也因此不对拒绝给恢复建议。
    - [x] `from_legacy` 在**唯一一处**做前缀分类，让 8 个工具无需各自重写即可迁移。
- [x] **27.3 统一管线**：`parse args → policy gate → deadline → execute → audit`。
    - [x] per-tool timeout：默认 120s，`run_shell` 600s（构建与测试套件本就要几分钟，
          两分钟杀掉一次真实构建比等它更糟）；超时返回 `TimedOut` 并提示改用后台。
    - [x] ⚠️ **未采用 `ToolImpl` trait**（原 27.1）。8 个工具、5 个阶段，插件式中间件栈
          只会增加间接层而永远不会被重新配置。真正重要的是**只有一条路径**，
          任何工具都绕不开任何一个阶段——一个有序函数已经保证了这点。
          MCP（阶段二十八）需要动态工具时再引入注册表，届时它才有真实用户。
- [x] **27.4 `ask_user_question` 工具**（L2-7）
    - [x] 交互模式走 stderr，与 `approval` 的 y/N 同一通道；数字选项 / 自由文本都支持。
    - [x] headless 下**立即拒绝**并告诉模型「选一个合理默认值并说明假设」——绝不挂起。
- [x] **27.5 测试**（12 个）：拒绝不算失败 / 前缀不重复 / 遗留文本分类 / 错误链保留 /
      未知工具是失败不是 panic / shell 超时更长 / **暂停时钟下验证 `sleep 3600` 真的触发超时** /
      ask 的非交互拒绝与缺参报错。

**顺带修正回放的形状校验**：新增工具让所有 fixture 失效，但对话 digest **逐字相同**
（`6d82789e5913f7af`）——只是多了一项模型没用到的能力。工具集因此降为**告警**而非硬失败：
硬失败会让每次新增工具都要重录整套 fixture，却抓不到任何东西。消息条数与最后一条角色
仍是硬失败，因为那才是「回放的答案属不属于这段对话」的判据。

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

本表是**所有取舍级缺口的完整清单**——评估里每个未被阶段二十 ~ 三十三承接的
缺口编号都必须出现在这里，由 `scripts/check-gap-coverage.py` 在 CI 中强制。

| 缺口 | 项 | 理由 |
| --- | --- | --- |
| L3-5 | OS 沙箱（Landlock / Seatbelt / bwrap） | 威胁模型不同；需要隔离请在容器内运行 |
| L2-6 | 持久 PTY / `terminal_*` 工具族 | `run_shell` + 后台 job 覆盖 90% 场景 |
| L2-8 | `todo_write` 工具 | PLAN.md / TODO.md 文件约定已覆盖，且天然跨压缩持久 |
| — | Code Mode（`run_code`） | 收益在超大工具面时才显现 |
| L5-5 | workflow / DAG 编排 | 是另一个产品 |
| — | 跨会话语义记忆 | 与 CLI 即时性目标背离 |
| L5-3 | 插件框架 / profile / bundle | 扩展需求由 MCP 承担 |
| L5-4 | hooks 协议桥 | 待 MCP 落地后重估——多数 hook 需求可用 MCP server 表达 |
| L6-3 | ACP / JSON-RPC server | 待 `--output json` 落地后重估，多数集成需求它已覆盖 |
| L6-4 | TUI / Web UI | 与本地 CLI 定位冲突 |
| L7-3 | 快照回归测试框架 | 阶段二十五的录制回放已覆盖主要回归需求，再加一套是重复投资 |
| L7-5 | OTel 导出 | 单机无 collector；JSON span + `jq` 已够 |
| L8-2 | 模型自排程（`schedule_*`） | 需要持久调度状态机；外部 launchd + 声明式 TASK.md 已覆盖 |
| — | 文档 i18n | 单人中文开发 |

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
