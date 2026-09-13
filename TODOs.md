# SeekCLI 进化路线图（阶段二十起）

> **定位**：单纯的本地 CLI Agent，核心 = DeepSeek + Tools + Harness Agent 引擎。
> **心智模型与逐层设计**：[`docs/architecture/`](docs/architecture/README.md)
> **缺口从哪来**：[`docs/evaluation/`](docs/evaluation/README.md)（首评定义缺口编号，三评新增 D 口径与六条新缺口）
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

## 进展（2026-08-23）

阶段二十 ~ 三十三**全部推进完毕**，其中三项经评估后**主动不做**并写明理由
（30.4 后台子代理、31.2 LoopHook、25.3 压缩轨迹 fixture），阶段三十三的
crates.io 发布已于 2026-09-12 定为不做，多平台 release 的 tag 仍待打。

| 指标 | 阶段十九 | 现在 |
| --- | --- | --- |
| 单测 | 87 | **207** |
| 行覆盖率 | 46.3% | **~68%**（`engine.rs` 13.8% → 65%） |
| 内置工具 | 8 | **14** + 任意 MCP |
| 编译警告 | — | **0** |
| CI 覆盖率门禁 | 无（装了不跑） | 60% 棘轮 |

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

### ✅ 阶段二十八：L5 MCP 客户端（**打开生态**）

*目标：让第三方能给 SeekCLI 加能力，而不必改 Rust 源码。*
*来源：L2-1 / L5-1 —— 评估认定的**最大单点缺口**。*
*设计：[L5 §4.1](docs/architecture/L5-composition.md#41-mcp-客户端l5-1)*

- [x] **28.1 协议实现**
    - [x] **只做 stdio transport**；手写 JSON-RPC，零新依赖。拥有失败模式才好处理：
          server 不应答、乱序应答、把日志打进 stdout 污染帧流——SDK 恰恰会藏起这些。
    - [x] 读响应按 id 匹配并跳过无关行（通知、banner），而非假定下一行就是回复。
    - [x] `isError` 与 JSON-RPC error 分开：前者是工具说「不」，后者是 server 坏了。
    - [x] `initialize` → `notifications/initialized` → `tools/list` → 注册。
- [x] **28.2 配置**：`config.toml` 的 `[[mcp]]` 数组（name / command / args / env / enabled）。
- [x] **28.3 健壮性**
    - [x] **单个 server 失败不影响启动**：警告 + 跳过（外部进程不可靠是常态）。
    - [x] 启动超时上限（默认 10s/server），避免拖慢 REPL。
    - [x] 进程随 REPL 退出而回收（`Drop` 里 kill，否则每次运行每个 server 泄漏一个进程）。
- [x] **28.4 安全与命名**
    - [x] 命名空间 `mcp__<server>__<tool>`——两个 server 都可能提供 `search`。
    - [x] MCP 工具**默认按非只读处理**，除非 server 明确 `readOnlyHint`。
    - [x] 一律过阶段二十四的策略门。⚠️ **首次实现有真实漏洞**：`is_mutating` 只按内置
          工具名匹配，`--read-only` 下 `mcp__fs__write_file` 真的把文件写出来了（端到端
          验证抓到）。修法是反转判定——我们没写的东西，除非 server 声明只读否则当作会写；
          并删掉不带声明参数的便利重载，那正是会被顺手误用的不安全默认。
    - [x] MCP 工具只给主 agent：子代理模板的 `allowed_tools` 是在不知道用户配了什么的
          前提下写的，悄悄放宽会破坏「工具收窄」这个让子代理又便宜又安全的机制。
- [x] **28.5 `/tools` 命令**：列出当前生效工具及其来源（内置 / MCP，含所属 server）。

**真实端到端验证**（`npx @modelcontextprotocol/server-filesystem`）：
- 同时配一个真 server 与一个命令写错的：14 个工具接入，坏的只打印一条警告不阻塞启动
- 模型真实调用 `mcp__fs__read_text_file` 并拿到文件内容
- `--read-only` 下 `mcp__fs__write_file` 被拒且**磁盘验证文件未创建**（修复后）
- `--read-only` 下 `mcp__fs__read_text_file` 仍可用——只读模式没有变成「全禁」

---

## 🟡 P2：产品化与深水区

### ✅ 阶段二十九：L7 评估闭环强化

*目标：让「引擎变好还是变坏」有数可依。*
*来源：L7-1。设计：[L7 §4.2](docs/architecture/L7-observability.md#42-eval-套件扩容l7-1)*

- [x] **29.1 eval 套件扩到 26 个任务**：`fs.json`(6) / `shell.json`(4) / `multistep.json`(5) /
      `recovery.json`(4) / `safety.json`(4) + 原 `basic.json`(3)。
    - [x] 单测强制「所有随包套件都能解析、任务有名有 eval、总数 ≥20」——
          手写 JSON 的笔误否则要等到有人花真金白银跑 benchmark 才暴露。
- [x] **29.2 反向断言**：`expect_fail: true` 让 eval 命令保持陈述句形式——
      写成取反的 shell 表达式会把意图埋进 `!` 和 `test` 里。
    - [x] 配套 `flags`（如 `--read-only`）把「在哪个模式下测」留在套件内，而不是
          取决于运行器怎么被调用。没有 flags 的反向断言会在普通模式下悄悄通过，
          所以有单测强制两者同时出现。
- [x] **29.3 Cost / Trace 小改**
    - [x] 费率移进 `config.toml [pricing]`：published 费率会变、按模型不同，
          硬编码只会变成一份「悄悄算错」的账单而不是一份明显错的。
    - [x] trace span 增加 `verdict`（acted / answered / empty）、`content_bytes` 与本轮工具名单。
          阶段十九是靠人肉读 trace 发现的，命名之后下一次可以直接 grep。

**顺带修掉两个真实缺陷**（都是做本阶段验证时撞出来的）：
1. **headless 下 tracing 完全不工作**：`run_headless` 从不调用 `start_run` / `flush`，
   `-p` / `--bench` / `--run-task` 一条 trace 都不产生——而那恰恰是没人盯着终端、
   最需要 trace 的模式。
2. **`-p` 遇到「已打开但不产出」的 stdin 管道会永久挂住**：从脚本里启动且 stdin
   未关闭时，进程无输出无报错地卡死——正是阶段二十三立誓要消灭的失败模式。
   改为在侧线程读取、2 秒无数据即放弃并告警；实测空管道不再挂住、真实管道数据仍读得到。

**待真实跑分**：`seekcli --bench examples/benchmarks/<suite>.json` 需要真实 API 与成本，
留给你按需执行。套件本身的结构正确性已由单测覆盖。

---

### ✅ 阶段三十：L2 后台任务与 job 控制（30.4 另行排期）

*目标：长命令与子代理不再阻塞对话。*
*来源：L2-5 / L5-2。设计：[L2 §4.3](docs/architecture/L2-tools.md#43-后台任务l2-5)*

- [x] **30.1 job 注册表**：输出写 `~/.seekcli/jobs/<id>.log` 而非内存缓冲——一次长构建
      不能无界撑大进程，`job_output` 也才能只取尾部。stderr 与 stdout 写同一日志：
      构建的报错正是模型需要的，分流会藏起它们并丢掉先后顺序。
    - [x] 随 REPL / headless 退出清理，**不做守护进程**；日志纳入 30 天清扫。
- [x] **30.2 工具**：`run_shell(background)` / `job_list` / `job_output(tail)` / `job_kill`。
    - [x] `job_output` 截断时**显式说明省略了多少行**——一个看起来完整的尾部会让模型
          断定某个报错从没发生过。
    - [x] `job_list` / `job_output` 进只读并发白名单；未知 id 返回 `[FAILED]` 而非 panic。
- [x] **30.3 完成通知**：在 step **顶部**注入，不打断当前 step——一次构建结束不该把模型
      正在做的事打断。且**只播报一次**：每轮重复是唠叨不是信息。
- [ ] **30.4 后台子代理**：`invoke_agent(background)` —— **本阶段未做**。
      后台 shell 与后台子代理只是共享「注册表」这个词，实际机制不同：前者是子进程 +
      日志文件，后者要在同一进程内跑一整条 agent 循环并把事件写进独立 session。
      硬塞进同一次改动只会让两者都变形。留作独立小阶段。

**真实端到端验证**：模型后台运行 `sleep 3; echo BUILD_DONE` → 立即返回并用
`job_list` 看到 running → 完成通知自动注入 → `job_output` 读到输出。全链路一次跑通。

**顺带**：job 注册表是进程级的，job 测试的完成通知会泄漏进回放测试并改变消息条数。
已并入 `crate::testsync` 单锁——与 cwd / 策略模式 / 审批模式同一类问题。

---

### ✅ 阶段三十一：L1 循环拆解与取消传播（31.2 暂不做）

*目标：把 440 行主循环拆成可组合、可单测的阶段。*
*来源：L1-1 / L1-2 / L1-3 / L1-5。设计：[L1 §4](docs/architecture/L1-engine.md#4-目标设计)*

- [x] **31.1 拆解主循环**：**524 → 339 行**，行为零变化，由回放测试守住 + 子代理真实回归。
    - [x] 抽出 `request_step`：把一条 delta 流变成一个响应。它没有自己的控制流，
          渲染 / usage 记账 / 流内中断检查都属于这件事而非循环。
    - [x] 抽出 `delegate_to_subagent` / `activate_skill_by_name`：引擎级委派工具，
          不走 dispatcher（前者会重入循环，而 dispatcher 刻意不认识循环）。
    - [x] ⚠️ **未按设计稿切成 `prepare_step` / `observe`**：剩下的不是「可以搬走的整块」，
          而是循环自身的控制流（迭代上限、中断、plan_next、reminder 触发）。硬切只会把
          控制流分散到多个函数、靠参数传状态，读起来更难而不是更容易。
- [ ] **31.2 `LoopHook` trait** —— **暂不做**，理由同阶段二十七不做 `ToolImpl`：
      抽象要有第二个用户才立得住。现在挂在循环上的 compressor / reminders / tracer /
      job 通知 / recovery 全是内部的、编译期已知的、各自已有单测；套进
      `Vec<Box<dyn LoopHook>>` 换来一层间接而非任何新能力——**没有第三方插件要挂进来**。
      值得做的信号是「出现一个需要在循环里插手、又不该进 engine.rs 的东西」。
      MCP 挂在工具层、L8 在循环外，都没产生这个需求。
- [x] **31.3 取消传播**（L1-5）：`run_shell` 改为 `spawn` 而非 `.output()`，`tokio::select!`
      在 `child.wait()` 与取消之间竞争，取消时 `start_kill` 并回收。
    - [x] 管道并发抽干：子进程写满管道缓冲区后若没人读会永久阻塞，所以不能先等退出。
    - [x] 取消时**不清除中断标志**：清了会杀掉命令却让循环若无其事地继续。
    - [x] 部分输出仍交给模型（走 offload），并标记 `[USER DENIED]` 明确「别重试」。
    - [x] **实测**：SIGINT 后 `sleep 120` 进程数 2 → 0（此前会一直跑完）。
- [x] **31.4 外部事件注入**（L1-3）：阶段三十的后台任务完成通知已经是这条路径——
      在 step 顶部注入、不打断当前 step。没有再单独造一个 `inject()` API：
      目前只有一个生产者，为它加一层间接不划算。

**验收**：SIGINT 后子进程实测 2 → 0；200 单测与回放测试全过；子代理委派真实回归通过。

---

### ✅ 阶段三十二：L8 任务声明式化

*目标：加一个定时任务不再需要改 Rust。*
*来源：L8-1。设计：[L8 §4.1](docs/architecture/L8-loop.md#41-任务声明式化l8-1)*

- [x] **32.1 `TASK.md` 格式**：与 `SKILL.md` 同构，**共用同一个 frontmatter 切分器**
      （`skills::split_frontmatter`）——两者都是「YAML 头 + 一段其实是 prompt 的 Markdown
      正文」，在 BOM 或 CRLF 上各自漂移毫无意义。
    - [x] `skill: null` 与省略该行等价；正文为空直接报错（正文就是 prompt）。
- [x] **32.2 `task_spec` 改为读 `~/.seekcli/tasks/<name>/TASK.md`**；找不到时报错并列出可用任务。
    - [x] 新增 `seekcli task list`：一个坏掉的定义只让那一行显示「无法解析」，
          不让整张表读不出来。
- [x] **32.3 内置 reminders 首次运行或首次 `task list` 时自动写出 `TASK.md`**。
      让它走与用户自定义任务**完全相同的路径**——否则两条路会漂移，而只有一条被真正跑过。
- [x] **32.4 `seekcli task install <name>`**：按 `interval_hint`（`15m` / `2h` / `1d` / 裸秒数）
      生成 plist 到 stdout，**不提供自动安装器**——往用户 LaunchAgents 塞东西应当是显式动作。
    - [x] `task` 子命令只需要 config，不构造 provider——查看任务不该要 API key。

**真实验证**：`task list` 首次调用自动写出内置 reminders；随后**只写一个 `TASK.md`**
就多出一个 `standup` 任务，无需改代码或重编译；`task install standup` 依据
`interval_hint: 1d` 正确生成 `StartInterval=86400` 的 plist。

---

### 🟡 阶段三十三：分发与发布（crates.io 已定为不做，tag 待打）

*目标：让第二个用户装得上。*
*来源：评估 §4 产品形态。*

- [x] **33.1 多平台 release**：`build.yml` 拆成 `check` + `release` 两个 job。
      门禁只在 Linux 跑一次（四平台跑同一套检查只会让 CI 时间翻四倍去重复证明同一件事）；
      tag 时构建 macOS(arm64/x86_64) + Linux(x86_64/arm64) 四份产物并上传 tar.gz。
    - [x] 附 SHA-256 校验和：让下载可以被验证，而不必信任提供下载的那个页面。
    - [x] 此前 release 只发 changelog、没有任何产物——「发布」等于「写了篇说明」。
- [x] **33.2 `cargo install seekcli` 可用**：补齐 description / repository / homepage /
      readme / keywords / categories / rust-version。
    - [x] `include` 只打包 `src/` + README + LICENSE + CHANGELOG：录制的 LLM fixture
          是 76K 测试数据、`examples/` 是文档，都不该进已发布的 crate。已用
          `cargo package --list` 核对。
- [x] **33.3 README 重写**：开头改为面向新用户——一句话说清它是什么、怎么装、
      五条常用命令，然后才是设计定位。「刻意不做什么」直接列出并指向理由所在。
- [x] **33.4 CONTRIBUTING + issue 模板**。
    - [x] CONTRIBUTING 首先讲「先读哪份设计文档」，并说明 `check-gap-coverage.py`
          会强制「评估 → 设计 → 路线」这条链不断。
    - [x] 单独一节讲「改动 LLM 交互时要格外小心」——这类 bug 不会编译失败也不会 panic。
    - [x] bug 模板引导附 `SEEKCLI_TRACE` 决策树，并提示 `verdict: answered` +
          `tool_calls: 0` 意味着模型只是在说话；同时提醒 trace 含提示词内容。
    - [x] feature 模板先问「是否已被明确排除」与「能不能用 MCP 解决」。

> **2026-09-12 决定：不发布到 crates.io**（仓库所有者）。
> 33.2 的元数据工作不作废——`cargo install --git` 依赖同一份 `Cargo.toml` 字段，
> 且它是将来若改主意时的前置。README 的安装指令已同步改为 `--git` 形式。
>
> ⏳ **仍待执行**：`git tag v0.1.0 && git push --tags` 触发多平台 release。
> 不打 tag 则 README 里「从 Releases 下载二进制」这条路同样不存在。
>
> 注意 `Cargo.toml` 的 `rust-version = "1.85"` 是按 edition 2024 估的下界，
> 若要严格保证，需要用该版本工具链实测一次。

---

## 🟣 第二轮：自进化地基（阶段三十四 ~ 三十九）

*坐标系：D 口径（自进化七条件）。来源：[2026-09-12 三评](docs/evaluation/2026-09-12-self-evolution-baseline.md)。*

**定位补充（不改主定位）**：SeekCLI 是 RL / 进化实验的**环境与评测器**，不是训练器。
理由：完全复用已有 L7 基建，训练端留在外部，不违反 design-principles §2 任何一条。

> ⚠️ **开工前置**：本轮六个阶段的「设计」环节尚未落到 `docs/architecture/` 层文档。
> 按 `docs/evaluation/README.md` 的「评估 → 设计 → 路线」链路，
> 每个阶段动手前需先在对应层文档补一节目标设计。另有四处文档修订待做，见本节末。

### 依赖关系

```
34 教学式错误（L1-6）        ← 零架构改动，随时可做
35 自描述（L7-6）            ← 零架构改动，随时可做

36 SessionStore seam（L4-7）─┬─→ 38 eval 回灌（L7-7）
37 提案闸门通用化（L5-6）────┘
                             └─→ 39 trajectory 导出（L7-8）
```

**三十四、三十五互相独立且不依赖任何人**，是本轮唯一可以立刻开始的两项。

---

### 🟣 阶段三十四：L1 教学式错误

*目标：每一次拒绝都变成模型能据此行动的下一步，而不是一句「不允许」。*
*来源：L1-6。设计：[L1 §4.6](docs/architecture/L1-engine.md#46-教学式错误l1-6)*

样板已经存在——`src/tools/mod.rs` `execute_with` 的 `bad_args` 分支注释写着
「Surface it explicitly so Error Recovery can hand the model an actionable hint」。
本阶段是把这个已经做对一次的模板推广到另外三处。

- [x] **34.1 策略门拒绝带上下文**（L1-6）：`src/tools/policy.rs` 的 `Verdict::Deny(reason)`
      补「当前 mode 允许什么」与「建议的替代调用」。
    - [x] 不改判定逻辑，只改拒绝消息的信息量——**判定与措辞必须分开改**，
          否则一次改动同时动了安全语义和文案，回归时说不清是哪边坏的。
    - [x] `shell_is_read_only` 的 yes/no 换成 `read_only_obstacle` → `NotReadOnly`
          五态，求值顺序与原谓词逐行一致；既有 207 测试原样通过即判定未变的证明。
    - [x] 顺带修掉 `classify_command` 丢弃定罪子命令的问题（§4.6.4）。
    - [x] 失去调用者的 `shell_is_read_only` 按零警告基线删除，yes/no 视图移入测试。
- [x] **34.2 MCP 启动失败记账并在拒绝路径讲出来**（L1-6）：`src/mcp/mod.rs` 现在是
      跳过 + 一行告警，模型完全不知道少了哪些工具、为什么少。
    - [x] 仍然保持「不静默降级」——可见输出照旧（`record_failure` 既 warn 又记账）。
    - [x] **不加常驻 system prompt 段**：失败记在 `McpRegistry.failures` 上，模型真的
          去碰那个不存在的工具时由 `unknown_tool_error` 讲出来。常驻成本为零，且不与
          §4.6.5「只有 `[MODE DENIED]` 该改 prompt」打架。详见设计 §4.6.6。
    - [x] 三条失败路径（connect / timeout / list_tools）统一走 `record_failure`。
    - [x] 未知名分三种答法：服务器启动失败（带原因 + 「重启是用户的动作」）、
          无任何 MCP 工具、名字打错（列出真实可用的）。
    - [x] `failures()` 访问器**未保留**——阶段三十五有调用者时再加，
          同 34.1 删除 `shell_is_read_only` 的判断。字段已就位。
- [x] **34.3 未知工具名附最接近的候选**（L1-6）：`registry::unknown_tool_message`
      给出最接近的可用工具名 + **调用签名**而非 schema 全文（错在名字或漏参数，
      签名一眼可见；schema 全文又长又答非所问）。
    - [x] 建议有下限：编辑距离超过名字长度 1/3 就不给建议，改列全部可用工具。
          **一个几乎不沾边的候选比不给建议更糟**——它让模型带着虚假信心走错路。
          `mcp__*` 形状的名字因此天然落到「列清单」那支。
    - [x] Levenshtein 手写（两行滚动），不引依赖：错误路径上的一个短函数
          不值得一个 crate，`cargo deny` 也少一件要表态的事。
    - [x] 签名区分必填与可选：`run_shell(command, [background])`。
- [ ] **34.4 eval 增补**（L1-6）：三条反向断言——拒绝消息必须包含可行动信息，
      而不只是断言「被拒绝了」。

**验收**：L-a 层进化（把知识写进拒绝路径而非 system prompt）在三条路径上成立；
A 口径的 recovery 质量可由 eval 前后对比证明。

---

### 🟣 阶段三十五：L7 自描述（`harness_inspect`）

*目标：模型能查询自己的运行时，而不是靠盲试推断边界。*
*来源：L7-6。设计：待补（`docs/architecture/L7-observability.md`）*

dsh 的教训写在它的 Agent Note 里：模型猜方法签名、猜返回值形状要花很多步盲试。
**自描述的收益先于自修改兑现**——即使永远不做自修改，这一条也值。

- [ ] **35.1 只读工具 `harness_inspect`**，一个 `what` 参数分区返回：
    - [ ] `tools`：当前工具面 + schema（含 MCP 来源标注）
    - [ ] `policy`：当前 mode 与**生效中的路径 / 命令规则**
    - [ ] `skills`：活跃 skill 与待审提案
    - [ ] `mcp`：各 server 状态与失败原因
    - [ ] `session`：事件日志统计（事件数 / 压缩次数 / 当前 token 估算）
- [ ] **35.2 与策略门同源**：`policy` 分区必须读 `policy.rs` 的同一份规则，
      **不得另写一份描述**——手写描述会漂移，且漂移时模型信的是错的那份。
- [ ] **35.3 归入只读并行工具**：`is_parallel_readonly` 加白名单。
- [ ] **35.4 eval 增补**：模型被拒后应能用 `harness_inspect` 自行定位原因。

**验收**：七条件第 1 条从 ❌ 到具备；且 35.2 保证它不会成为第二份会漂移的事实来源。

---

### 🟣 阶段三十六：L4 会话存储 seam

*目标：把 `LlmProvider` 的 seam 模式从 L0 推广到 L4。*
*来源：L4-7。设计：待补（`docs/architecture/L4-memory.md`）*

**判据是「真的出现第二个实现」**——trajectory 导出（三十九）、SQLite 索引、
跨会话检索，任一个都够。这与当初推迟 `LoopHook` 用的是同一条判据
（见「🔵 已知仍开放」），区别只在于这次判据被满足了。

- [ ] **36.1 `trait SessionStore`**：`save` / `load` / `list` / `stat`。
    - [ ] 现有 JSONL 实现原样塞进去，**行为零变化**——这一步只做搬家。
    - [ ] `Session` 保持纯数据；`to_jsonl` / `from_jsonl` 降为该实现的私有细节。
- [ ] **36.2 全部调用点改走 trait 对象**，确认 `/resume`、`fork`、检索、标题
      四条路径无行为差异（录制回放回归）。
- [ ] **36.3 不引入第二个后端**——本阶段只开缝，不塞东西。
      塞东西是三十九的事，届时才验证这条缝开对了。

**验收**：`cargo test` 全绿且录制回放无 diff；新增 trait 但**不新增任何行为**。

---

### 🟣 阶段三十七：L5 提案闸门通用化

*目标：把已经做对的人工闸门从「只服务 skill」推广到全部可进化资产。*
*来源：L5-6。设计：待补（`docs/architecture/L5-composition.md`）*

七条件第 6 条（选择压力）是 SeekCLI **强于 dsh** 的两处之一——
dsh 的四档持久没有升档闸门，完全交回人工开发流程。本阶段扩大这个优势面。

- [ ] **37.1 提案类型泛化**：`proposals/` 下按类型分目录，`skill` 之外新增
      `mcp`（server 配置）、`policy`（策略规则）、`task`（TASK.md）。
- [ ] **37.2 `/skill accept|reject` 泛化为 `/propose list|accept|reject`**，
      保留 `/skill` 旧入口为别名——**不破坏既有肌肉记忆**。
- [ ] **37.3 每类提案有自己的落地校验**：policy 提案接受前必须能解析，
      mcp 提案接受前必须能启动一次。**接受一个坏提案比拒绝一个好提案贵得多。**
- [ ] **37.4 补全 completer**：`src/completer.rs` 已按 Tab 重扫目录，跟着泛化。

**验收**：写一个 MCP server 配置提案 → `accept` → 重启后工具面真的多出来，全程不改 Rust。

---

### 🟣 阶段三十八：L7 eval 回灌提案闸门 ★

*目标：把「可判定的评价信号」接到「选择压力」上——闭上进化飞轮。*
*来源：L7-7。依赖：三十六、三十七。设计：待补（`docs/architecture/L7-observability.md`）*

> **这是 dsh 做不到的事。** 它的 e2e / snapshot / web 测试在 CI 里跑，不在会话里跑。
> 单人本地 CLI 的 eval **可以在接受提案的那一刻当场跑完**——
> 体量劣势反过来变成结构优势。

- [ ] **38.1 `/propose accept` 前跑相关 eval 套件**，输出 before/after 对照。
- [ ] **38.2 提案可声明关联套件**；未声明则跑默认冒烟集。
      **默认必须是「跑一点」而不是「不跑」**——默认不跑等于这个功能不存在。
- [ ] **38.3 回归即拒绝**：任一 Fail-to-Pass 任务由通过变失败则拒绝接受，
      并把失败任务名告诉模型（教学式错误，与三十四同一原则）。
    - [ ] 提供 `--force` 逃生口，但**必须打印被牺牲了哪几条**。
- [ ] **38.4 eval 增补**：一个故意引入回归的提案必须被挡下。

**验收**：七条件第 5 条与第 6 条之间建立机械连接；进化从「可编程」变成「可判定」。

---

### 🟣 阶段三十九：L7 trajectory 导出

*目标：让 SeekCLI 成为可被外部 RL / 进化流程消费的环境。*
*来源：L7-8。依赖：三十六。设计：待补（`docs/architecture/L7-observability.md`）*

- [ ] **39.1 `--bench` 导出标准 trajectory**：`(state, action, reward)` 序列，
      reward 取 Fail-to-Pass 的退出码判定（含反向断言的取反语义）。
- [ ] **39.2 走 36.1 的 `SessionStore` 缝**——这是验证那条缝开对了的第一个真实用户。
- [ ] **39.3 明确不做训练端**：只产出数据，不引入任何训练依赖。
- [ ] **39.4 格式文档化**：外部消费者需要一份稳定契约，否则导出等于没导出。

**验收**：`events.jsonl` 之外产出一份外部可消费的轨迹；L7-8 关闭。

---

### 本轮附带的文档修订（未排期，动手前逐条确认）

按 P3 表的规矩——「若将来重估，需先修改对应设计文档再开阶段」——以下四处
**必须在相关阶段动手前完成**，本轮仅登记，不在此提交中修改：

| 文件 | 改什么 | 关联 |
| --- | --- | --- |
| `docs/architecture/design-principles.md` §2 | 「在线自演化 Skill」现写作排除项，实为七条件第 6 条**做对了**，表述反了 | 三十七 |
| `docs/architecture/design-principles.md` §5 | 补判据本质：dsh 是为不确定性买保险，SeekCLI 是为确定性优化 | 全部 |
| `docs/architecture/README.md` §5 | 「不借鉴 Cordis」升级为精确版：Rust 的 `Drop` + 所有权是 Fiber 754 行状态机的编译期版本 | 全部 |
| P3 表「插件框架 / profile / bundle」一行 | 取舍性质变化：不是「不做插件」，是**「MCP 就是我们的插件格式」** | 三十七 |

---

## 🔵 已知仍开放（未排期，但不是「不做」）

推进完阶段二十 ~ 三十三后，核对层文档与路线图发现这四项仍然开放。
它们既没做完、也没有被判定为「明确不做」——列在这里而不是悄悄留在层文档里，
是因为 `scripts/check-gap-coverage.py` 现在会强制两边一致。

| 缺口 | 现状 | 为什么没做 |
| --- | --- | --- |
| L1-1 | 主循环 524 → 339 行，但仍无正式扩展点 | `LoopHook` 需要第二个用户才立得住，见 31.2 |
| L1-2 | 无 turn / step 模型，只有 `iter` | 阶段三十一只做了拆解。引入 turn/step 要动 `LoopResult` 与事件模型，收益目前只有「概念更清晰」 |
| L1-4 | 中断后**上下文**可恢复，**任务**不会自动接着跑 | `/resume` 还原对话已够用；要真正接着跑需在投影里识别末尾 `Interrupted` 并合成继续指令，是个独立小改动 |
| L5-2 | 子代理一次性，无法续跑 / 通信 | 30.4 已主动推迟并写明理由；完整的可续子代理还需要 mailbox 与生命周期管理 |

**这四项不构成「路线图未完成」**：阶段二十 ~ 三十三的任务本身都已推进到位，
这些是那些阶段**有意留下的边界**，各自写了理由。什么时候值得做，见对应层文档的
「目标设计」。

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
| — | Cordis 式 DI / Proxy context | Rust 的 `Drop` + 所有权已是 effect 可逆性的编译期版本；Fiber 754 行状态机在 GC 语言里手工重建的东西，rustc 免费提供 |
| — | 运行期代码挂载（dlopen / WASM） | 跨 ABI 不安全、卸载几乎必然 UB；WASM 要拖进整个 runtime 且无法授予宿主级权限。等价需求由 MCP 的**进程边界**满足——kill 即完整 quiescence |
| — | 发布到 crates.io | 仓库所有者决定不发布。`cargo install --git` 与 Releases 二进制已覆盖安装需求；发布会引入版本号承诺与 yank 语义这类长期义务 |
| — | RL 训练端 | 定位是**环境与评测器**，不是训练器。训练端是另一套技术栈，塞进 Rust CLI 违反 design-principles §5 三问 |

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
