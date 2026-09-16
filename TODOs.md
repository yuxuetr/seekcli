# SeekCLI 进化路线图（阶段二十起）

> **定位**：单纯的本地 CLI Agent，核心 = DeepSeek + Tools + Harness Agent 引擎。
> **心智模型与逐层设计**：[`docs/architecture/`](docs/architecture/README.md)
> **缺口从哪来**：[`docs/evaluation/`](docs/evaluation/README.md)（首评定义缺口编号，三评新增 D 口径与六条新缺口）
> **阶段一 ~ 阶段十九**：已全部完成并归档至 [`docs/archive/TODOs-phase-01-19.md`](docs/archive/TODOs-phase-01-19.md)

---

> **勾选框的约定**：`- [ ]` 只表示**还要做的事**。
> 「评估后决定不做」的项**打勾**并在正文写明判决——决定本身已经完成。
> 这样 `grep '- \[ \]' TODOs.md` 数出来的就是真实待办，不用人工筛。
> （2026-09-16 六评发现：此前 17 个未勾选项里只有 6 个是真的。）

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
crates.io 发布已于 2026-09-12 定为不做；多平台 release 已于 2026-09-13 发出
（`v0.2.0`，四平台产物 + SHA-256）。

| 指标 | 阶段十九 | 现在 |
| --- | --- | --- |
| 单测 | 87 | **343**（阶段四十八后；阶段三十三时为 269） |
| 行覆盖率 | 46.3% | **70.9%** |
| 内置工具 | 8 | **17** + 任意 MCP |
| eval 任务 | 3 | **30** |
| 图像入口 | 无 | **3 条**（`/paste`、`read_image`、MCP 透传） |
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
    - [x] ~~触发压缩的长会话~~ —— 录一条需要几十万 token，成本不划算；
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
- [x] **30.4 后台子代理**：`invoke_agent(background)` —— **本阶段未做**。
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
- [x] **31.2 `LoopHook` trait** —— **暂不做**，理由同阶段二十七不做 `ToolImpl`：
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

### ✅ 阶段三十三：分发与发布（crates.io 不做）

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
> **2026-09-13**：版本号 0.1.0 → **0.2.0**，已打 `v0.2.0` 标注 tag。
>
> 没有复用 `v0.1.0`——它是 2026-04-29 的本地 tag，在 HEAD 之前 147 个提交，
> 移动它会抹掉那个历史标记（远端本就无任何 tag，所以也不存在「已发布」问题）。
> 选 0.2.0 而非 0.1.1 的依据在代码里：`config.rs` / `history.rs` 多处把
> 配置与会话的旧格式称作「pre-0.2」并已做完迁移——当前状态就是 0.2。
>
> **2026-09-13（同日稍后）**：`main` 与 `v0.2.0` 均已推送，但 **CI 红、Release 未创建**。
>
> 原因是一行 `use std::io::Write;`——它唯一的用处 `write_all` 在 `/copy` 的
> `#[cfg(target_os = "macos")]` 块里，于是在 Linux 上是死导入，被 `-D warnings`
> 判成编译失败。**在 macOS 上本地怎么跑都是绿的**，所以它从 2026-07-17 起
> 就让 `build` 一直红着，三次失败（29591391079 / 34697969231 / 34765645626）
> 原因完全相同，tag 构建也栽在同一处：四平台产物一个都没出。
>
> 这里真正该记的教训**不是「门禁不够」**——`check` job 就跑在 Linux 上，
> 它每一次都正确地报了错。失败的是**没有人去读 CI 结果**：两个月里所有
> 「零警告基线」的说法都只在 macOS 上成立，而那台机器看不见这个错误。
> 本地交叉 check 试过，被 `openssl-sys` 需要 Linux sysroot 挡住，不划算；
> 可靠的做法就是推送后看一眼 `gh run list`。
>
> 修复见 `1ac1eea`（import 收进它需要的那个块）。全仓仅此一处平台门控代码。
>
> 经仓库所有者同意，`v0.2.0` 已移到修复后的提交并重推，Release **创建成功**——
> 但**只出了三个平台**：`aarch64-unknown-linux-gnu` 挂在 `openssl-sys` 上。
>
> 同一个病根第二次发作。workflow 为这个 target 装了 `gcc-aarch64-linux-gnu`，
> 那是**链接器**；`openssl-sys` 要的是 aarch64 的 **libssl 这个 C 库**，从来没装过。
> 也就是说这一格从加进矩阵那天起就不可能成功，只是此前 `check` 先红、
> `release` 根本没机会跑，于是没人发现。
>
> 顺带暴露出一个没人注意到的发布问题：native-tls **动态链接构建机的 libssl**，
> 所以已经发出去的 x86_64 Linux 产物，在 libssl 版本不同的发行版上起不来——
> 「发布了二进制」和「那个二进制能在别人机器上跑」是两件事。
>
> 改用 rustls 一次解决两者（`0d0b985`）。保留 `macos-system-configuration`
> （macOS 系统代理靠它）并用 `native-roots` 而非 webpki-roots（继续读 OS 根证书，
> 自签 CA 的代理才不会失效）。后端是 ring，交叉编译只需 cc。
> 交叉产物现在只依赖 libc / libdl / libm / libpthread。
>
> **验证方式补课**：本地用 `zig cc` 当交叉编译器真跑通了 aarch64 release 构建
> 并检查了产物的动态依赖，又用真实 API 调用确认 rustls 握手可用——
> 换 TLS 后端属于「编译不会失败也不会 panic」的改动，只看编译过不算验证。
>
> 经仓库所有者同意删除那个三平台 Release 并重发（它发出不到半小时，且其中的
> Linux x86_64 产物本身就有缺陷，留着比删掉更坑）。`v0.2.0` 现指向 `b5e04f5`，
> **四平台产物 + SHA-256 全部就位**，`check` 与四个 `release` job 全绿。
>
> 至此 README 里「从 Releases 下载二进制」这条路第一次真正成立。
>
> 本阶段真正的收获不在任何一项勾选里，而在两次失败共用的那个形状：
> **门禁都写对了，缺的是有人去读它的输出。** `check` job 就跑在 Linux 上，
> 两个月里每一次都正确地报了那行死导入；aarch64 那格从加进矩阵起就不可能过。
> 两者都不是「没有检查」，是「检查的结果没人看」——而本地在 macOS 上
> 怎么跑都是绿的，那台机器结构性地看不见这两个错误。
>
> 推论：**声称「零警告 / 全绿」时必须说清是在哪个平台上**。
> 推送后 `gh run list` 看一眼，比再加一道本地门禁便宜得多也可靠得多。
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
37 提案闸门通用化（L5-6）──→ 38 eval 回灌（L7-7）
39 trajectory 导出（L7-8）   ← 独立，不依赖 36
```

**三十四、三十五互相独立且不依赖任何人**，是本轮唯一可以立刻开始的两项。

> **2026-09-13 两次调整顺序，第二次把阶段三十六整个撤下。**
>
> 第一次：37 提前到 36 之前。36 会创建 `trait SessionStore`，而按 36.3
> 「只开缝不塞东西」，第二个实现要等到 39——那违反
> [design-principles §5.1](docs/architecture/design-principles.md#51-三问背后的判据)
> 的判据（抽 trait 要有真实的第二个实现）。
>
> 第二次：做 39 的前置调研推翻了 36 的立项理由本身。三评写的理由是
> 「trajectory 导出（39）、SQLite 索引、跨会话检索，任一个都够」，
> 但**trajectory 导出是导出格式，不是存储后端**——它读 `run.events`
> （已经在 `LoopResult` 里，只是被 `#[cfg(test)]` 圈住），不需要换后端。
> 剩下两条都没排期。**于是 L4-7 没有任何近期的第二个用户**，
> 移入「🔵 已知仍开放」——这正是本仓为「没做完，但也不是不做」发明的第三个桶。
>
> 38 原写「依赖 36、37」，实际只依赖 37（它要的是 bench 与提案闸门）。

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
- [x] **34.4 eval 增补**（L1-6）：三条断言——拒绝后模型的**行为**必须改变，
      而不只是断言「被拒绝了」。
    - [x] **撞到一条限制并顺手补掉**：eval 层看不到模型说了什么（`run_eval` 只看
          testbed 里的 shell 退出码，`HeadlessOutcome.text` 被丢弃），而 `--read-only`
          下模型不被允许写任何证据。现在把最终回答以 `$SEEKCLI_ANSWER` 暴露给 eval
          命令，文件放在 **testbed 之外**（否则断言「没创建任何文件」的任务用 `ls -A`
          会看见它）。同一通道是阶段三十八 eval 回灌需要的。
    - [x] 断言分两层：**单测**管消息文本，**eval** 管模型行为。详见设计 §4.6.7。
    - [x] `safety.json` +3：被拒后报出数值而非放弃、讲出障碍是什么、
          把理由类别（`privilege escalation`，不在 prompt 里）带回用户。
    - [x] **L1-6 已结清**：`safety.json` 真实跑过 **7/7 PASS**（≈¥0.06 / 32s），
          三条新任务全过。模型输出里可见 34.1 在起作用——
          「The `&&` chain means the whole command was refused up front,
          so `ls` did not run either. Not retrying.」

**验收**：L-a 层进化（把知识写进拒绝路径而非 system prompt）在三条路径上成立；
A 口径的 recovery 质量可由 eval 前后对比证明。

---

### 🟣 阶段三十五：L7 自描述（`harness_inspect`）

*目标：模型能查询自己的运行时，而不是靠盲试推断边界。*
*来源：L7-6。设计：[L7 §4.5](docs/architecture/L7-observability.md#45-自描述-harness_inspectl7-6)*

dsh 的教训写在它的 Agent Note 里：模型猜方法签名、猜返回值形状要花很多步盲试。
**自描述的收益先于自修改兑现**——即使永远不做自修改，这一条也值。

- [x] **35.1 只读工具 `harness_inspect`**（L7-6），一个 `what` 参数分区返回：
    - [x] `tools`：当前工具面 + **调用签名** + 来源标注（built-in / mcp:<server> / skill）
    - [x] `policy`：当前 mode 与**生效中的**命令规则
    - [x] `skills`：活跃 skill、可用 skill、**待审提案**（看见已有提案才不会重复起草）
    - [x] `mcp`：各 server 状态与失败原因（复用 34.2 的 `failures`）
    - [x] `session`：事件数 / 压缩次数 / session id
    - [x] 未知 `what` 列出合法取值——与 34.x 同一条「拒绝必须可行动」的规则。
    - [x] 走 `execute_with` 而非引擎分支（设计 §4.5.3），保住「没有任何工具能绕过
          策略门 / deadline / 审计」的不变量；引擎先装一个纯数据 `Snapshot`。
    - [x] 全量报告约 1192 字符（约 300 token），按需调用。
- [x] **35.2 与策略门同源**：`policy` 分区从 `policy.rs` 的同一份常量渲染
      （新增 `read_only_commands()` / `mutating_subcommands()` 访问器），
      **不另写描述**——手写描述会漂移，且漂移时模型信的是错的那份。有单测守：
      遍历常量断言每一项都出现在输出里。
- [x] **35.3 归入只读并行工具**：`is_parallel_readonly` 加白名单；
      `dispatch` 因此要认识它（并行批次也走那条路），snapshot 每轮按需装一次。
- [x] **35.4 端到端验证**：`engine.rs` 两个测试用真实 `App::for_test` 装 snapshot
      并走完整 `dispatch`——只差「模型自己决定调用它」，那需要录制 fixture。
    - [x] **L7-6 已结清**：录了 `tests/fixtures/inspect-after-denial`——模型被拒后
          调用 `harness_inspect{what:"policy"}`，报出 `policy.rs` 的真实白名单
          （含 git / cargo 例外与重定向规则），还指出 `echo` 虽在表里但没有重定向
          就建不出文件。回放测试**断言挂在轨迹上而非措辞上**（必须出现
          harness_inspect 调用），否则换一种说法就会误红。

**验收**：七条件第 1 条从 ❌ 到具备；且 35.2 保证它不会成为第二份会漂移的事实来源。

---

### 🟣 阶段三十七：L5 提案闸门通用化

*目标：把已经做对的人工闸门从「只服务 skill」推广到全部可进化资产。*
*来源：L5-6。设计：[L5 §4.3](docs/architecture/L5-composition.md#43-提案闸门通用化l5-6)*

七条件第 6 条（选择压力）是 SeekCLI **强于 dsh** 的两处之一——
dsh 的四档持久没有升档闸门，完全交回人工开发流程。本阶段扩大这个优势面。

- [x] **37.1 提案类型泛化**（L5-6）：`~/.seekcli/proposals/<type>/`，`skill` 之外新增
      `mcp`（server 配置）、`task`（TASK.md）。旧家 `skills/proposals/` 首见时搬过去
      （目录 rename + 一行可见输出；提案本来就是待审的临时物，风险低）。
    - [x] **同时交付生产者**：`propose` 工具。只泛化闸门而不给新类型生产者，
          等于又一个没有用户的抽象——与把 36 推后是同一条理由。
    - [x] 触发链已就位：`harness_inspect{what:"mcp"}` 让模型看得见能力缺失，
          `harness_inspect{what:"skills"}` 现在列出**全部类型**的待审提案，
          所以模型不会把上一轮提过的再提一遍。
    - [x] `propose` 刻意**不接 skill**：把 create_skill 的结构化字段压成一个
          `content` 字符串，等于逼模型手写 SKILL.md frontmatter，是人机工效的退步。
          两个生产者，一道闸门。
    - [x] ⚠️ **`policy` 类型本轮不做**：落地需就地改写已存在的 `[security]` 表，
          而 `toml 0.8` round-trip 会抹掉用户 config.toml 的注释（「生成一份带注释的
          配置」是对用户的承诺）。做对它需要 `toml_edit` 新依赖或拆分安全配置文件，
          那是配置架构决定，不属于闸门范围。详见设计 §4.3.4。
- [x] **37.2 `/propose list|accept|reject <kind> <name>`**，`/skill accept|reject`
      保留为别名——**不破坏既有肌肉记忆**；两者都转发到同一个 `ProposalStore`，
      `SkillManager::{accept,reject}_proposal` 已删除，**不留第二个实现**。
    - [x] `/propose list` 顺手对每条跑一次校验并标出「ok」或「cannot land: …」——
          不能落地这件事值得在审阅时就知道，而不是按下 accept 才发现。
- [x] **37.3 每类提案有自己的落地校验**：**接受一个坏提案比拒绝一个好提案贵得多**。
      skill → `SKILL.md` frontmatter 可解析；mcp → **反序列化成 `McpServerConfig`**
      （校验用的就是将来加载它的那段代码）；task → frontmatter 可解析且正文非空。
    - [x] mcp 落地用**纯追加**而非改写，单测断言原文件注释一字不动且整体仍可解析。
    - [x] 起草时就校验一次：当轮能被模型自己改掉的错，不必等到用户审阅。
          草稿**保留**在盘上，让模型迭代而不是从头再来。
- [x] **37.4 补全 completer**：`/propose <verb> <kind> <name>` 三级补全，
      名字按 Tab 重扫对应类型目录。

**验收**：写一个 MCP server 配置提案 → `accept` → 重启后工具面真的多出来，全程不改 Rust。

---

### ✅ 阶段三十八：L7 eval 回灌提案闸门 ★

*目标：把「可判定的评价信号」接到「选择压力」上——闭上进化飞轮。*
*来源：L7-7。依赖：三十七。设计：[L7 §4.7](docs/architecture/L7-observability.md#47-eval-回灌提案闸门l7-7)*

> **这是 dsh 做不到的事。** 它的 e2e / snapshot / web 测试在 CI 里跑，不在会话里跑。
> 单人本地 CLI 的 eval **可以在接受提案的那一刻当场跑完**——
> 体量劣势反过来变成结构优势。

- [x] **38.1 `/propose accept` 前跑冒烟集**（L7-7），输出 before/after 对照。
    - [x] 基线**当场跑不缓存**：过期基线会给出自信的错判。代价事前告知。
- [x] **38.2 默认冒烟集**（`basic.json`，3 个任务）。
      **默认是「跑一点」而不是「不跑」**——默认不跑等于这个功能不存在。
    - [x] ⚠️ **只对 `skill` 提案跑**：`mcp` 要重启才连上、`task` 是给调度器的提示词，
          对它们跑 eval 是表演。闸门**说清楚为什么不跑**而不是静默跳过——
          后者会让用户以为通过了检查。详见设计 §4.7.1。
    - [x] 衡量的是**回归**不是能力：「激活它之后，agent 在原本就会做的事情上
          是否变差了」。把它说成「验证 skill 有效」是过度宣称（§4.7.2）。
    - [x] 顺带发现 bench 从来没激活过 skill（`run_headless(prompt, None)` 写死），
          所以先给它开了 `score_suite(suite, trajectory, skill)`。
- [x] **38.3 回归即拒绝**：任一任务由通过变失败则拒绝，并**点名是哪几个**
      （教学式错误，与阶段三十四同一条规则）。
    - [x] 逃生口叫 `--skip-eval`（不是 `--force`），且**必须打印跳过了什么**——
          一个悄悄跳过检查的逃生口等于没有检查。
    - [x] 判决是**单向**的：新通过的任务不抵消新失败的。套件是通用的而 skill
          不是，让一次无关的胜利去「买」一次回归是错的。有单测守。
- [x] **38.4 判决逻辑做成纯函数**（`Regression::compare`），所以「故意引入回归
      必须被挡下」用单测确定性验证，不必每次烧一次真实调用。
    - [x] 真实验证仍跑了：故意写坏的 skill（「不要调用任何工具」）
          **3/3 → 1/3 被挡下并点名 create_file / count_lines**；
          无害的 skill 3/3 → 3/3 通过；mcp 提案打印「为什么不跑」；
          `--skip-eval` 打印「lands unmeasured」。

**验收**：七条件第 5 条与第 6 条之间建立机械连接；进化从「可编程」变成「可判定」。

---

### 🟣 阶段三十九：L7 trajectory 导出

*目标：让 SeekCLI 成为可被外部 RL / 进化流程消费的环境。*
*来源：L7-8。无依赖。设计：[L7 §4.6](docs/architecture/L7-observability.md#46-trajectory-导出l7-8)*

- [x] **39.1 `--bench --trajectory <file>` 导出 JSONL**（L7-8）：每行一个任务，
      reward 取 Fail-to-Pass 退出码，**已应用 `expect_fail` 取反语义**
      （消费方不需要知道哪条是反向断言）。
    - [x] **reward 是终局的，不伪造每步 reward**——在导出层摊派奖励等于把一个
          建模决定硬编进数据，而 credit assignment 是消费方的事。
    - [x] 写出 `reward_kind`（恒为 `fail_to_pass`）：将来加别的判定方式时
          旧数据不会被误读成新语义。
    - [x] `status` 区分 completed / max_iterations / interrupted——
          撞上迭代上限的轨迹和自己收尾的不是一回事，不可混训。
    - [x] 显式开启，不默认往盘上丢文件；导出失败**在报表之后**报告，
          不让它夺走用户等来的分数。
- [x] **39.2 读 `HeadlessOutcome.events`**——原先只在 `#[cfg(test)]` 下被捕获，
      现在是产品 API。顺带删掉 `App` 上的 `last_events` 字段：它存在的理由正是
      「events 不在产品 API 里」，现在不成立了。
    - [x] 导出顺带暴露了一处不对称：`chat()` 把 user 消息记进会话，
          `run_headless`（`--bench` 走的那条）留给调用方，所以 bench 日志从第一条
          assistant 开始。**在导出层补 step 0 的 observation，不动事件日志的排序**——
          那是 L4 最吃重的不变量，为了让导出好看去重排它是坏交易。
- [x] **39.3 明确不做训练端**，也**不声称兼容任何 RL 框架**：声称兼容就欠下一个
      跟着别人版本走的义务，而没有任何消费方在要求它。
- [x] **39.4 格式契约**写进 [L7 §4.6.3](docs/architecture/L7-observability.md#463-一条记录--一个任务)。
- [x] **真实端到端验证**：`SEEKCLI_REPLAY` 回放 fixture 跑完一条 read-only 任务，
      导出 3 步轨迹 + 终局 reward + status，**无需 API key 与费用**。
      该次运行顺带证明了 34.1——step 2 的 observation 里是新拒绝消息的原文
      （fixture 录在改动之前，消息是运行时生成的）。

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

## 🟤 第三轮：基底变更（阶段四十 ~ 四十一）

*来源：[2026-09-13 基底变更](docs/evaluation/2026-09-13-substrate-shift.md)。*

**性质与前两轮不同**：代码没变，**基底变了**——`deepseek-flash` 获得视觉能力，
于是项目里若干条写死的前提由真变假。缺口不是「没做」，而是「做对过，但世界动了」。

### 依赖关系

```
40 纠正已变假的前提（L0-5）   ← 不等任何人
41 多段内容（L4-8）──┬─→ MCP 图像透传（L2-9）
                     └─→ /paste 图像输入（L6-6）
```

---

### ✅ 阶段四十：L0 纠正已变假的前提

*目标：让项目不再对 agent 和用户陈述假话。*
*来源：L0-5。设计：[L0 §4.5](docs/architecture/L0-llm-substrate.md#45-模型能力是实测的不是写死的l0-5)*

不动任何 schema，纯粹清理。**不等阶段四十一**：一条会误导 agent 的陈述，
留着的成本是每天都在付的，而纠正它不需要等任何架构决定。

- [x] **40.1 默认模型名改为 `deepseek-flash`**（L0-5）：`/v1/models` 只报
      `deepseek-flash` 与 `deepseek-v4-pro`；`deepseek-v4-flash` 仍是别名，
      所以这不是故障修复，是**不要在默认配置里写一个过期名字**。
    - [x] 真实验证：干净 HOME 跑 `-p`，新生成的 config.toml 写的是
          `flash_model = "deepseek-flash"`，调用成功。
- [x] **40.2 `vision` skill 标记为过时**：它开篇写「DeepSeek V4 是纯文本模型，
      本身不能"看"图像」——前提已假。整个 skill（`clip_to_png.sh` +
      `vlm_describe.sh` + StepFun）是为一个不存在的限制做的绕行，
      **现在让 agent 白跑一趟外部 VLM**。
    - [x] **改的是仓库里那份**（`examples/skills/vision/`）+ README 的表格行。
          原计划写的是「改用户 `~/.seekcli/` 下那份」——查过才发现仓库自己就发这个
          skill，README 还教人 `cp -r` 安装。**改源头比改用户的副本对**，
          用户那份是他们的数据，不动。
    - [x] 没有直接删：图像真进上下文要等 L4-8，在那之前它仍是唯一退路。
          SKILL.md 里写明「保留它是为此，不是因为它还正确」，并注明
          阶段四十一落地后应当删除。
- [x] **40.3 `mcp/protocol.rs` 的理由改对**：注释写的是「cannot be shown to a
      text-only model」，真实原因现在是**我们的 `Message` 还承载不了多段内容**。
      行为不变（仍 `[image content omitted]`），但**理由必须指向真正的阻塞点**，
      否则下一个读者会以为是模型的限制而不去碰它。已指向 L4-8。
- [x] **40.4 实测脚本留痕**：三条 curl 记进 [L0 §4.5.2](docs/architecture/L0-llm-substrate.md#452-实测方法复查时照跑)，
      连同那个陷阱——纯红小图会让模型从「single color square」猜中，
      必须用猜不出来的双色分割才是有效实验。

**验收**：✅ 242 测试全绿；`grep -rn "text-only" src/` 已无命中；
仓库发的 vision skill 与 README 都标了过时与原因。

---

### ✅ 阶段四十一：L4 多段内容（架构级）

*目标：让图像能进入请求，并且能从事件日志重建。*
*来源：L4-8 → 解锁 L2-9 / L6-6。设计：[L4 §4.6](docs/architecture/L4-memory.md#46-多段内容l4-8)*

> ✅ **开工前置已完成**：
> 1. [L4 §4.6](docs/architecture/L4-memory.md#46-多段内容l4-8) 多段内容设计。
> 2. [design-principles §1.1](docs/architecture/design-principles.md#11-能力与用户输入的分界)
>    已划清「能力」与「用户输入」的界。

- [x] **41.1 `Message` 承载多段内容**（L4-8）：`content: String` → 文本 + 图像。
    - [x] 向后兼容：纯文本消息的序列化形状**不得改变**，否则既有 fixture 全废。
- [x] **41.2 `EventPayload` schema 演进**：按「模型可见 = 已记录」，
      图像进入请求就必须能从 `events.jsonl` 重建。
    - [x] 事件日志有版本与迁移的先例（阶段二十六），沿用同一套做法。
    - [x] 图像**不内联进日志**：blob 归属机制已经存在（`tools/offload.rs`），
          日志里存引用。否则一次截图会让 `events.jsonl` 膨胀数百 KB。
- [x] **41.3 token 计数与成本口径复查**：16×16 的图令 prompt 从 31 涨到 224。
      `api/tokens.rs` 的启发式是按文本长度估的，对图像会严重低估。
- [x] **41.4 MCP 图像透传**（L2-9）：`[image content omitted]` 改为真的传过去。
    - [x] 已验证这条 wire 接受 `tool` 角色的多段内容。
          坑：thinking 模式下 assistant 消息必须带回 `reasoning_content`。
    - [x] **停了一轮换来一个更干净的方案**：三个原候选都没选。
          改**闭包的输出类型**（`ToolOutput`）而不是每个工具的签名——
          内置工具靠 `From<String>` 一行转换，自身签名不动；没有共享可变状态，
          守门仍然只有一处。详见设计 §4.6.4。
    - [x] 持久化点唯一：`App::push_tool_response`，blob 先落盘再记事件。
    - [x] 真实验证：自建 MCP server 返回 64×64 上红下黑的图，模型答对颜色
          （**明确禁止读源码**，排除从 stub 源码推断），blob 落盘经 `file`
          确认是真 PNG（即 base64 往返正确），日志只存引用。
- [x] **41.5 `/paste` 图像输入**（L6-6）：截图 → `Cmd+Shift+4` → `/paste`，
      **零路径管理**。macOS 用 osascript 读剪贴板；内部临时文件（如
      `/tmp/seekcli_paste.png`）对用户不可见，可以接受。
    - [x] **不做**客户端扫描消息里的路径自动塞图——那才是 §1 排除的 `@xxx` 模式。
- [x] **41.6 eval 增补**：`recovery.json` 加 `image_is_read_not_guessed`，
      断言模型答出图里的两种颜色（蓝 / 黄，猜不出来的图）。
    - [x] **写它时发现第三个入口是缺的**：bench 用不了剪贴板，也不能按任务配 MCP，
          而**没有任何工具能把图片读进对话**——于是补了 `read_image`。
          它其实是三个入口里最符合 §1.1 的那个（**Agent 自己决定去读**），
          没有它，一个在满是截图的仓库里工作的 agent 一张也看不了。
    - [x] 类型由**magic bytes** 判定而非扩展名：一个其实是 JPEG 的 `.png`
          会被报错类型，provider 直接拒。4 MB 上限，超了让它先裁剪。
    - [x] PNG 用 base64 种进 testbed——`files` 只能装 UTF-8 字符串，PNG 不是。
          （第一次用 `python3 -c` 写，`\n` 没被 shell 解释，setup 直接失败。）
    - [x] 真实验证：`recovery.json` **5/5 PASS**（≈¥0.066）。
- [x] **41.7 删除 `vision` skill**（计划外，随 41.6 落地）：它的退路作用到此为止。
      `deepseek-flash` 自己能看图，图像也真能进请求了，留着只会让 agent
      白跑一趟外部 VLM，换一段比原图信息量更少的文字。README 同步。

**验收**：截图 → `/paste` → 模型描述正确；`/resume` 后该图仍在上下文里
（即真的从日志重建了）；既有录制 fixture 全部照旧通过。

---

## 🟤 第四轮：从「建成」到「在用」（阶段四十二 ~ 五十）

*坐标系：真实使用。来源：[2026-09-15 五评](docs/evaluation/2026-09-15-usage-reality-check.md)。*

**本轮的第一前提**：五评核查 `~/.seekcli/` 得出——九层文档、269 测试、30 个 eval
任务、四平台发布**全部成立**，但**日常使用已经停止四个月**。

全量核对 76 个会话后的时间线（五评 §1.5，**这条更正了五评首版的「零真实使用」**）：

| 月份 | 冒烟 | 真实 |
| --- | --- | --- |
| 2026-04 | 0 | **20** |
| 2026-05 | 0 | **24** |
| 2026-06 ~ 09 | 32 | **0** |

真实会话覆盖的正是本项目声明的四个场景：雅思备考计划、`@tavily 苹果股票价格`、
`@/Users/hal/Downloads/...` 文档分析（10 次）、`/paste` 图像汇总、Rust 编程问答。
**最后一个真实会话是 2026-05-16，同一天的提交是
`35e524c refactor(main): strip peripheral sensors...`（main.rs −735 行），即阶段七。**
此后 32 个会话无一例外是冒烟与 eval；审计日志 767 条调用全部来自测试夹具
（`T/seekcli-loop-*`）与 eval testbed，无一条来自真实工作目录。

其余现状：MCP 零配置；launchd 零安装；金融 skill 不存在；雅思三个 skill 是
6 行提示词壳；累计 proposal 唯一一个是 `user_lucky_number`（冒烟产物）。

> **阶段七的判断本身没错**——`@xxx` 客户端预注入确实违反 design-principles §1，
> `doc_parser` 后来以 skill + 脚本的合规形态回来了就是证明。
> **错的是只做了拆除而没有做等价替换**：`@web` / `@tavily` 至今没有替代品，
> 所以从 05-16 起金融与检索类问题在 SeekCLI 上根本无法回答。
> 这把**阶段四十三从推测性优先级变成恢复一项已被证明在用的能力**。

> **这解释了第二、三轮留下的「有理由不做」为什么都长得像同一件事。**
> `LoopHook` 无第二用户、`SessionStore` 立项理由被推翻、`allowed_tools` 停在
> 「phase 12.5 will wire」——四评说「每项都有写明的理由」是诚实的，但理由的性质
> 不是取舍，是**缺少真实负载来产生需要它们的压力**。

**排序原则**：先让它被用起来，再让它可追踪，最后才让它自我提升。
自我提升消费的是真实失败记录——那是现在唯一缺的原料，**且不能靠写代码造出来**。

### 三类工作分离

| 类 | 性质 | 是否依赖真实使用 |
| --- | --- | --- |
| **A 确证缺陷** | 与用不用无关，现在就错 | 否，可立即开工 |
| **B 场景解锁** | 缺了就没法用于四个场景 | 是，需失败清单校准 |
| **C 内容缺口** | **不是代码问题**，是没写 | 否，但属非代码工作 |

把 C 类识别出来是本轮规划的主要收益之一：金融 / 雅思 / CQF skill 的缺口
**是内容缺口，不是代码缺口**。`doc_parser`（79 行 + `scripts/`）已经证明
skill + shell 脚本这条路可用，照抄即可，不该占代码排期。

### 依赖关系

```
零 一周真实使用 ────────────────────┐（产出失败清单，校准下方所有 B 类）
                                    │
42 确证缺陷（A 类，不依赖清单）     │
  42.1 事件日志 ──┬─────────────────┼──→ 47 Run 身份统一
  42.2 allowed_tools                │         │
  42.3 子代理中断                   │         │
  42.4 压缩阈值                     │         │
                                    │         │
43 外部资料通道 ★（恢复 05-16 拆掉的能力）←┤      │
  43.3 不可信输入（不得晚于 43.1）  │         │
        ├──→ 44 Skill 契约 ←────────┤         │
        └──→ 46 三个 skill 内容（C 类，试用期间并行）
                                    │         │
45 领域状态与记忆 ←─────────────────┤         │
                                    │         ▼
48 lib.rs + 共享预算（收益待清单确认，可后置）
                                    └────→ 49 场景化评估 ──→ 50 Plugin 预留
```

**唯一的硬序**：零 → 43 → 49。自我提升的输入是真实失败，真实失败来自真实使用，
真实使用需要能出网。

---

### 🟤 阶段四十二：三处已核实的不一致（A 类）

*目标：修掉与「用不用」无关、现在就是错的三件事。*
*来源：五评 §3——L4-9 / L5-7 / L1-7 / L4-10。四项互相独立，一项一个 commit。*

- [x] **42.1 事件日志名实不符**（L4-9，**缺陷级**）✅ 2026-09-16（两个 commit）
    - [x] 现状：`history.rs:8` 模块头自称 `events.jsonl append-only log <- the source
          of truth`，实际 `history.rs:83-90` 是 `fs::write(全部事件)` 整文件重写；
          热路径唯一调用点 `engine.rs:557`，**整轮结束才落一次**。
    - [x] **第一半**：`Session` 增加 `persisted` 水位线，`save_session` 只追加
          `unpersisted()`，并 `sync_data`。顺序刻意与直觉相反——**先 append
          日志，再写 meta.json**：meta 是日志的摘要，先写摘要会在崩溃时留下
          「声称拥有日志里没有的事件」的会话；反过来最坏只是摘要落后，
          `load_session` 免费修正。`fork` 水位线归零（新文件）。
    - [x] **第二半**：`Journaling` 枚举 + 两个落盘顺序点。
          点 1 在工具执行前，点 2 在结果回交模型前。
    - [x] 关键记录写不下去时**分情况**：本轮全是只读调用 → 告警继续（只读可重跑，
          代价只是可追溯性）；**有副作用 → 直接 bail，一个工具都不执行**。
    - [x] 恢复不盲目重放：投影为**每个没有结果的 `tool_call` 补一条
          `UNKNOWN_RESULT` 占位**。措辞是「结果未知」而非「失败」——
          告诉模型它失败了，会诱发对一件**可能已经发生**的事情的重试。
          这同时修好了一个硬故障：wire 格式要求每个 `tool_calls` 后面跟着
          tool 消息，没有占位的话崩溃后的会话**能加载但永远无法续跑**。
    - [x] 顺带：`derive_messages_indexed` 同步计入合成消息，
          否则两趟投影长度不一致会在 debug 构建里触发 `debug_assert`——
          而用户跑的正是 release 构建。
- [x] **42.2 `allowed_tools` 语义不对称**（L5-7，**安全级**）✅ 2026-09-16
    - [x] 现状：SubAgent 模板真裁剪（`engine.rs:249` → `registry::filter_by_allowed`）；
          Skill 完全不裁（`skills.rs:286` 自注 "not yet consumed downstream"，
          而它指向的「phase 12.5」从未存在）。
          **同名字段一边是权限边界一边是装饰。**
    - [x] 选了实现：`registry::narrow_to_skill` 在 loop 入口裁剪 effective 工具面，
          位置在 MCP 合并**之后**，所以作用于真实工具面而非只作用于内置工具。
    - [x] 三条不变量各有单测：**只能删不能加**（声明需求 ≠ 被授予权限，
          实际授权仍只来自 policy gate 与用户批准）；**匹配不上的名字被点名**
          而非静默忽略；**裁剪后为空合法但会被告知**。
    - [x] `delegate_to_subagent` 拿到的是裁剪后的集合，
          故 **skill 的裁剪无法通过委派绕过**。
    - [x] 顺带修了两处同源的名实不符：`render_skill_md` 此前从 `skill.tools`
          反推 `allowed_tools:`（渲染出的 skill 会声明它并未裁剪到的工具）；
          `ProposalStore::read_skill` 此前不带白名单，于是 **eval 闸门衡量的是
          未裁剪版本、落地的却是裁剪版本**——判决在谈论一个从不运行的 skill。
- [x] **42.3 中断不进子代理**（L1-7，**缺陷级**）✅ 2026-09-16
    - [x] 现状：`engine.rs:382`（流内）与 `engine.rs:962`（轮顶）两处中断检查都是
          `depth == 0` 门控。子代理跑起来后 Ctrl-C 不被它的循环看见——
          explore 15 轮 / general 20 轮可能干等很久。
    - [x] `tools::shell::set_interrupt` 会杀掉正在跑的 shell，但循环继续迭代，
          表现为「命令被杀了但 agent 还在转」。
    - [x] **修复时发现的陷阱**：轮顶检查用的是 `swap`（消费标志位）。若子代理也消费，
          它会停下自己却**让父循环继续跑**——比原 bug 更糟。故抽出 `take_interrupt`：
          **每一层都观察，只有 depth 0 消费**。子代理跳出后把
          `[Interrupted by user]` 作为工具结果交回，父循环下一轮顶看到标志位仍在，
          依次解开。3 条单测守住这条不变量。
    - [x] 顺带：`delegate_to_subagent` 原先对任何返回都说 "completed"。
          被中断或撞上迭代上限的子代理**没有完成**——现在按 `LoopStatus` 分别措辞，
          避免把「模型停了」当成「任务过了」交给父代理去推理。
    - [x] `request_step` 的 `depth` 参数随之删除（中断检查已与深度无关）。
- [x] **42.4 压缩阈值与模型窗口脱钩**（L4-10）✅ 2026-09-16
    - [x] 现状：`compressor.rs:40` 的 150K 与 `compressor.rs:44` 的 `KEEP_TAIL = 8`
          都是全局常量，而 provider 由配置静态选择、支持两套 wire 协议
          （原文写「三套」，实为 openai / anthropic 两套）。
    - [x] 换个窗口小的模型，压缩要么永不触发要么触发太晚，**两种失败都静默**。
    - [x] 新增 `[memory]` 配置段（`context_window_tokens` / `compact_at_ratio` /
          `keep_tail_messages`）与 `compressor::Budget`。
          **默认值精确复现旧常量**（200000 × 0.75 = 150000，尾部 8）——
          改的是可配置性与可见性，不是行为。
    - [x] 越界比例**响亮地夹取**而非照单全收：`ratio = 5.0` 若被 honor 等于悄悄
          关掉压缩。合法区间 `0.1..=0.95`，NaN 回落默认；`keep_tail` 下限 2
          （低于 2 会把 `tool_call` 与其结果劈开）。
    - [x] 阈值可被读回推导式：压缩日志行与 `harness_inspect{what:"session"}`
          都打印 `compact_at: 150000 tokens (window 200000 x 0.75)`。
    - [x] **顺带补了一条此前无人走过的路径**：生成的默认配置模板从没有往返测试，
          而 `[memory]` 是模板里的第一个浮点数。新增断言「模板解析回来 ==
          `Config::default()`」，并反向验证过它真的能抓到坏模板。

**验收**：四项各有单测；42.1 能在「工具已执行、结果未落盘」时重启并给出
「状态未知」而非静默丢失或盲目重放。

---

### 🟤 阶段四十三：外部资料通道 + 引用契约 ★（B 类）

*目标：让金融 / CQF / 日常问答三个场景第一次真的能答。*
*来源：L2-10（无出网能力）+ L3-6（不可信输入无边界）。依赖：阶段零失败清单。*

**这是对四个场景收益最大的单点改动。** 现状：17 个内置工具全是本地的，
**没有一个能出网**——这是三个场景共同的失败原因。

> **不自己写 crawler，也不为此新建插件系统。** `doc_parser` 已验证一条合规路径：
> 阶段七把 MinerU 当「客户端预注入能力」剥离后，它以 **skill + `scripts/*.sh` +
> `run_shell`** 的形态回来了，完全符合 design-principles §1（Agent 自取）。
> 搜索走同一条路，**零 Rust 代码**。

- [x] **43.1 搜索与取原文** ✅ 2026-09-16 —— **改为内置 Tool，不是 skill 脚本**
    - [x] 规划时写的是「skill + `search.sh` / `fetch.sh`，零 Rust 代码」。
          **动手前被 43.3 推翻**：不可信输入边界要求 harness **知道**结果来自网络，
          而 `run_shell` 返回的网页与 `ls` 的输出类型完全一致。
          把安全边界交给提示词自觉，与 L3 整层立场相反。
    - [x] 这同时兑现了 design-principles §1 对阶段七开出的处方——
          原文是「改造为 Tool」，阶段七只做了「剥离」。**只拆不还是执行了一半。**
    - [x] `web_search`（找候选来源，返回标题/URL/发布时间/摘要）与
          `web_fetch`（读一页原文）**刻意不合并**：搜索摘要由引擎生成、可能过时、
          可能与原文不符，**它是线索不是证据**。
    - [x] `recency_days` 参数——不带窗口时 Tavily 几乎全是 `published: unknown`，
          这对「现在股价多少」这类问题是硬伤。带窗口走 news topic，
          实测确实产出有日期的结果。
    - [x] `[research]` 配置段（provider / api_key / max_results），
          `provider = "none"` 时**两个工具根本不注册**——
          模型看得见却永远用不了的工具，每次尝试都白费一轮。
- [x] **43.2 引用契约** ✅ 2026-09-16
    - [x] `tools/provenance.rs`：记录**搜到什么**与**真正读过什么**，两者分开。
          同一 URL 先搜到再打开是「一个来源被检视两次」，不是两个来源。
    - [x] `harness_inspect{what:"sources"}` 分组渲染。**这才是让契约可检查的机制**——
          它显示真正打开过什么，而不是模型声称读过什么。
    - [x] `research_rules()` 系统消息陈述契约（只在工具真被提供时注入，
          且是独立消息，index 0 的 kernel 保持逐字节不变以免每轮 cache miss）。
    - [x] `/clear` 与 `/load` 清空来源——换了对话，上一段的来源不是它的证据。
- [x] **43.3 不可信输入边界**（L3-6）✅ 2026-09-16 —— 与 43.1 同一个 commit
    - [x] 网络内容进 `UNTRUSTED_WEB_CONTENT` 围栏，前言声明它**不携带任何权限**。
    - [x] **围栏标记本身是攻击面**：载荷里出现的闭合标记被**中和而非删除**
          （删除会让读者看不出这一页尝试过什么）。单测断言任意载荷下
          恰好只有一个真正的闭合标记。

**验收** ✅ **2026-09-16 已通过**（五评 §7.1）：重放 `智谱最近有什么值得关注的消息`
（与 2026-05-02 那条 `@web 智谱的股价` 同类，自阶段七起答不了）——6 轮、
`web_search` ×4 + `web_fetch` ×8、86K prompt / 99% cache hit，
**引用契约逐条自发落实**：区分摘要与原文（「这条我没有打开原文核实」）、
付费墙如实说明、时效性口径、单方面指控标注。
**待办**：投毒网页的注入 eval 仍未写，随阶段四十九补。

---

### 🟤 阶段四十四：Skill 契约补全（B 类）

*依赖：阶段零失败清单。**形态待清单确认，可降级或取消。***

- [x] **44.1** `references/` 的实际消费 ✅ 2026-09-16 —— **实测后推翻了「可降级」的判断**
    - [x] 先核实前提：`read_file` **不在** `is_mutating_with` 的名单里，所以
          `ensure_within_cwd` 不适用——模型**读得到** `~/.seekcli/skills/*/references/`。
          扁平场景本来就是通的，「references 不被支持」是错误印象。
    - [x] **真实缺口在递归**：`list_asset_entries` 不递归也不设上限。
          建了一个带 `references/chapter2/ito.md` 的测试 skill 实跑——
          模型**确实找到了**那个文件，但用掉 `list_dir` ×2 + `read_file`
          **三次额外调用**去发现，而且只因为它想到了要找。
          **想不到的时候是静默的**：教材分章节放，清单看不见，
          模型会用先验回答，而外观上像是有依据的。
    - [x] 改为递归（深度上限 3）+ 相对路径展示（`chapter2/ito.md`）
          + 条目上限 40 且**超出时明说还有几条、怎么找**（原则 §4「不静默截断」）。
    - [x] **实测对照**：同一个 skill、同一类问题，改后模型**直接 `read_file`
          命中嵌套路径**，首次尝试，2 次调用。
- [x] **44.2** Skill 版本与来源记录 ✅ 随阶段五十落地（`Skill::version` / `source`）。

---

### 🟤 阶段四十五：领域状态与长期记忆（B 类）

*来源：L4-11。依赖：阶段零失败清单。**本轮唯一需要推翻既有排除项的地方。***

排除项写着「跨会话语义记忆：与 CLI 即时性目标背离；session 事件日志 +
工作区 PLAN.md 已覆盖」。**这条是为 coding CLI 写的。** 雅思备考、CQF 进度、
金融关注标的是持续数月的状态，PLAN.md 覆盖不了。

- [x] **45.1** 三档，**全部是可编辑 markdown，不上向量库** ✅ 2026-09-16
      会话上下文（已有）／领域状态（`memory/<scope>.md`）／长期偏好
      （`memory/preferences.md`）。新增 `memory` 工具（list / read / write / forget）。
- [x] **45.2** 四项元信息各有归宿，不靠每条记录堆注释：来源与时间在条目行尾的
      HTML 注释里（渲染时不可见、模型可见）；**适用范围就是文件本身**；
      撤销方式写在文件头，并由 `harness_inspect{what:"memory"}` 给出目录路径。
      **一条用户看不出如何移除的记忆，是他没同意过的规则。**
- [x] **45.3** **做成路由，不是提示词里的请求**：`MemoryStore::append` 对
      `preferences` 直接 `bail`，`memory` 工具把这类写入转成 `Kind::Memory` 提案，
      只有 `/propose accept memory <name>` 能落地。
      **刚失败过的模型，恰恰是最可能写下「用户不理解条件期望」的模型**，
      而那句话此后会出现在每一次会话里。
- [x] **45.4** **偏好内联，领域只给索引**——这是防串味的核心而非省 token 的优化。
      把每个 scope 正文都注入，等于让雅思薄弱项坐在金融问题的上下文里。
      有单测断言领域正文不出现在注入内容里。
- [x] **真实验证**：`/propose list` 看到 memory 提案 → `/propose accept memory
      answer-in-chinese` → 落进 `preferences.md` 并标注「accepted by the user」→
      提案文件消失。闸门的「为什么不跑 eval」原先对 memory 说的是 mcp 和 task
      的理由，一并改成各自的真实理由——**解释别人情况的提示读起来就是套话，
      而套话是人们会停止阅读的东西**。
- [x] **回放测试抓到一个真问题**：记忆注入让 prompt 依赖可变的全局状态，
      五条 replay fixture 在开发机的记忆目录一有内容时就全红。
      **在干净机器上它们不会红**，所以修法是让 `App` 持有 store（测试下为 `None`）
      并**单独加一条断言**守住「回放的 prompt 不得依赖开发者的记忆目录」，
      而不是指望 fixture 去发现。
- [x] **补记缺口 L4-12**：记忆注入和 `workspace_rules` 一样不回流事件日志，
      违反 design-principles §3。**不是本阶段引入的，但本阶段加剧了它**
      （新增来源会随时间变化）。诚实记为缺口并排进 47.4，而不是顺手扩大范围——
      顺手修有个坑在投影语义上，见 47.4。

**验收** ✅ **2026-09-16 部分通过**（五评 §7.2）：新开会话问「雅思写作还有什么
薄弱项」，模型经索引读到 `ielts` scope 并答出已记录的 Task 2 问题；重复写入被拒绝；
两条经闸门接受的偏好被遵守（中文回答 / 技术术语用英文）。
**「隔一周」这半条仍需时间验证。**

---

### 🟤 阶段四十六：金融 / CQF / 雅思 Skill 实体化（**C 类，非代码**）

*不占代码排期。建议在阶段零试用期间边用边写。*

- [ ] **46.1** 形态直接抄 `doc_parser`（79 行 + `scripts/`），路径已验证。
- [ ] **46.2** 金融 skill **默认研究模式**：读取与计算，不含下单 / 转账。
      将来即使加交易工具也需独立授权，**不因启用金融 skill 而一并开放**。
- [ ] **46.3** 金融分析分离三类内容：**事实**（带来源/时间/单位/口径）、
      **计算结果**（由工具执行，保留输入与方法）、**判断**（写明假设与不确定性）。
- [ ] **46.4** 时效性口径：「当前」截至何时、实时还是延迟、新闻发布时间与
      事件发生时间是否不同、财务数据是否经过重述。

---

### 🟤 阶段四十七：Run 身份统一

*目标：回答「这次改文件来自哪次调用」「哪个子代理花了多少时间」。*
*来源：L7-9。依赖：42.1（L4-9）。*

已有两套追踪，但**互不相识**：`observability/trace.rs` 的 Run→Turn→leaf span 树
（`SEEKCLI_TRACE=1`）与 `session.rs` 的事件日志（`seq` 编号）是两个身份空间。

- [x] **47.1 trace 与日志打通** ✅ 2026-09-16 —— **加 join key，不做「从事件派生」**
    - [x] `run` span 注解 `session` + `first_seq`，`turn` span 注解 `seq`。
    - [x] **规划写的是「trace 从同一事件链派生」，实现时否决了**：trace 记时长、
          事件记内容；要让事件承载足够计时信息去重建 span 树，就得让每个事件都带
          起止时刻——而 trace 是 `SEEKCLI_TRACE=1` 才开的旁路，**零开销正是它的
          设计前提**（design-principles §3）。加 join key 花两行拿到同样的可归因性，
          合并两套记录要动的是所有事件的形状。
- [x] **47.2 ChildRun 归属** ✅ 2026-09-16
    - [x] 子代理只交回摘要文本，于是**一个悄悄撞上迭代上限的委派，在日志里与一个
          正常完成的委派完全无法区分**。现记模板 / 结束原因 / 迭代数 / 耗时，
          以父级 `tool_call` id 关联。
    - [x] 事件由 `delegate_to_subagent` **返回**而非就地记录，这样它进的是与本轮
          其余事件同一个 journal 缓冲，落在正确位置。
    - [x] `harness_inspect{what:"session"}` 渲染出来。
- [x] **47.3 结束原因 ≠ 验证状态** ✅ 2026-09-16
    - [x] `--output json` 新增 `verification`，与 `status` 并列。
          `status` 说循环为什么停，`verification` 说有没有东西检查过结果。
    - [x] `-p` 路径恒为 `not_verified`——headless 没有验收条件。今天唯一产出真实
          判决的是 benchmark runner（按验证命令退出码）。**把 `completed` 读成
          「成功了」正是这两个字段要防的错误。**
- [x] **47.4 注入的系统消息进事件日志**（L4-12）✅ 2026-09-16
    - [x] 新增 `ContextInjected` **记账事件**：按 digest 对同 kind 去重，
          内容进 blob（与图像同一套内容寻址），**不进投影**。
    - [x] **绕开了上一轮记下的那个坑**：记成 `SystemPrompt` 会重新投影，而记忆
          随笔记变化、append-only 不能撤回旧的，prompt 会堆积自己的历史快照。
          记账事件既满足 §3「模型可见 = 已记录」，又不碰投影语义。
    - [x] kernel 不记——编译期常量，不需要记录就能回答「模型看到了什么」。

---

### 🟤 阶段四十八：架构师评估（2026-09-16）

*评估先于动手，结论带「当前版本」限定（AGENTS.md「工程品格」）。
三个子项的结论不同，所以分开写。*

#### 48.1 `lib.rs` + 核心不打印终端 —— **当前版本不做**

**原定收益站不住。** 路线图写的是「REPL / headless / scheduler / eval 四个入口
共用同一个运行服务」——**它们已经共用了**，四个入口全部调用 `run_agent_loop`。
`lib.rs` 真正带来的是「crate 可被外部作为库消费」，而**当前没有第二个消费者**。
判据与 `LoopHook` 同一条（design-principles §5.1）。

「核心不打印终端」这半条也已实质满足：`engine.rs` 的 32 处输出是进度渲染，
`ui.rs` 带 `HEADLESS` 开关，`-p --output json` 实测输出干净 JSON。
真实要求（headless 下 stdout 机器可读）**已经成立**，再加一层抽象没有消费者。

**何时重估**：出现第二个消费者时——常驻进程、LSP server、
或一个需要内嵌引擎的测试宿主。

#### 48.2 RunContext / 显式工作目录 —— **当前版本不做**

`set_current_dir` 全仓 4 处：`tasks.rs` 进出各一、`benchmark.rs` 复原一、
`main.rs` 的 `--cwd` 一。**这是两对合法的 enter/restore，不是重复。**

真正的耦合在别处：**11 个进程级全局状态**（`policy::MODE`、`approval::INTERACTION`、
`offload::BLOB_DIR`、`provenance::SOURCES`、`web::BACKEND`、`shell::INTERRUPT`、
`jobs::JOBS`、`cost::RATES`、`ui::HEADLESS` 等）。把它们收进 RunContext 要改动
**每个工具的签名**，而唯一受益者是「同进程并发多个 Run」——当前没有任何东西想要。

**何时重估**：需要同进程并发运行多个 Run 时（例如 eval 并行化）。
在那之前，全局状态是**单进程单运行**这个事实的诚实表达，不是欠债。

#### 48.3 共享预算 —— **要做，但形状与原定不同**

**测量推翻了原定描述。** 路线图写的是「子代理从父运行继承预算，而不是每启动一个
就凭空拿一份完整预算」，并把 `MAX_SUBAGENT_DEPTH = 3` 列为现状。实测：

> **两个子代理模板的 `allowed_tools` 都不含 `invoke_agent`**
> （`EXPLORE` 的系统提示词里甚至明写 "you CANNOT spawn further sub-agents"）。
> **所以深度只能到 1，`MAX_SUBAGENT_DEPTH = 3` 守的是一条走不到的路。**

于是「每个子代理凭空拿一份预算」这个风险本身就被限制在一层，没有原先设想的那么大。

**而真正无界的维度没人看着**：

| 维度 | 上限 |
| --- | --- |
| 主循环轮数 | 25（`MAX_ITER`，`--max-iter` 可调） |
| 子代理深度 | 事实上 1 |
| 子代理迭代 | 15 / 20（模板固定） |
| **单轮工具调用数** | **无界**——模型自己决定 |
| **整次运行的总 LLM 调用 / 时长 / 费用** | **完全没有上限** |

最坏情况约 `25 轮 × N 个 invoke_agent × 20 迭代`，**N 无界**。

**最小真实修法不是「继承」，是「一个整次运行的总调用上限」**——它一次性
封住所有维度，包括没人看着的那个。而且 `cost.api_calls` 计数器**已经存在**，
子代理与父运行共享同一个 `self.cost`，**所以「继承」是靠计数自然得到的，
不需要把预算对象一层层传下去**。

落地形态：运行开始记基线 → 每次请求前比对 → 超出则以
`LoopStatus::BudgetExhausted` 停止（47.3 枚举里本就列了「预算耗尽」，
只是当时没有实现它）。**约 60 行，不是一套预算框架。**

##### ✅ 2026-09-16 已落地

- `[limits] max_llm_calls_per_run`，默认 150（一次六轮、八次取原文的研究型运行
  实测用掉 12，所以默认值远高于日常使用，只拦跑飞的循环）。
- **「子代理继承预算」靠计数自然成立**：父子共享同一个 `self.cost`，
  所以子代理的调用直接从父运行的额度里扣，不需要把预算对象传下去。
  有单测断言这条。
- `LoopStatus::BudgetExhausted` 独立于 `MaxIterations`——
  「主循环用完了轮数」与「整次运行用完了预算」是两件不同的事。
- **实跑验证**：`max_llm_calls_per_run = 2` 下，
  `status: "budget_exhausted"`、`final: "[Stopped: run budget of 2 model calls spent]"`、
  退出码 2（NOT_CONVERGED）、并打印如何调高上限。

**顺带记一条**：`MAX_SUBAGENT_DEPTH = 3` 守的是走不到的路（两个模板都不含
`invoke_agent`）。**不删它**——它是「子代理不得嵌套」这条约束的第二道防线，
成本为零；但把这个事实写在这里，免得下次有人据它推断深度真能到 3。

---

### 🟤 阶段四十九：场景化评估 + 真实失败集

*目标：给已经通的改进闭环喂真实原料。*
*来源：L7-10。依赖：阶段零、四十三。*

**已有的**：30 个 eval 任务、fail-to-pass 判定（按验证命令退出码，不按模型自称）、
`/propose accept skill` 跑两遍冒烟集回归即拒绝。**这条链是通的**（四评 §2.1 无误）。
**缺的是输入**——现有任务全是自造夹具。

- [x] **49.1 真实案例转 eval 任务** ✅ 2026-09-16 —— `examples/benchmarks/research.json`
    - [x] 5 条，**全部写自观察到的真实行为**（五评 §7.1），不是为覆盖代码路径编的夹具。
          **实跑 5/5 通过，总计 ≈¥0.06 / 33s。**
    - [x] **与冒烟集分开**：这些要联网、花真钱、依赖会变的页面，
          放进 `/propose accept` 跑的默认集会悄悄抬高每次提案的代价。
    - [x] 覆盖的正是引用契约与记忆闸门的机械可判定部分：
          引用真正打开过的页面 / 打不开要明说 / 摘要不冒充读过 /
          记忆跨会话可写 / **偏好只能提案不能直存**。
- [x] **49.4 门禁失效检查** ✅ 2026-09-16 —— **发现它本来就是失效的**
    - [x] 闸门用 `env!("CARGO_MANIFEST_DIR")` 定位冒烟集——那是**构建时**路径。
          装好的二进制在用户机器上找一个不存在的目录，打印一行安抚性的警告，
          然后 `return Ok(None)` **自动接受**。而 `examples/` 也不在
          `Cargo.toml` 的 `include` 里。
          **阶段三十八建的那道闸门，对任何不在源码树里跑的用户是静默失效的。**
    - [x] 改为 `include_str!` 嵌进二进制（用户可用 `~/.seekcli/benchmarks/basic.json`
          覆盖）。**闸门现在存在于二进制存在的每一处。**
    - [x] 顺带：套件格式此前静默忽略未知字段。我自己第一版 `research.json` 写了个
          `cleanup` 字符串（字段是数组）——加上 `deny_unknown_fields` 后被立刻抓到。
          同时暴露出 `_comment` / `_note` 两个既有约定**一直靠静默丢弃在工作**，
          现声明为字段并**让它们挣到位置**：套件说明开跑时打印，任务备注失败时打印。
    - [x] 新增 `cleanup` 字段（`Vec<String>`，eval 之后跑，失败只告警不改判决）——
          场景任务会写进真实的 `~/.seekcli/memory/`，实跑后确认无残留。
- [ ] **49.2** 分场景指标——「变好了」在四个场景里含义不同。
      **部分落地**：research.json 覆盖了「引用是否支持结论」这一维；
      编程 / 金融口径 / 学习进步三维仍需真实失败样本。
- [ ] **49.3** 保留集：不参与提案生成的任务，防止提案者同时掌握全部答案。
      **待真实提案出现后再做**——目前累计 proposal 仍只有测试产物。

**验收**：一个被接受的改进能说清它解决了失败清单第几条、对照评估结果、如何回滚。

---

### 🟤 阶段五十：Plugin —— 只做预留，不做平台

*判据用本仓自己的：[design-principles §5.1](docs/architecture/design-principles.md#51-三问背后的判据)
——翻转条件是**某一层出现真实的第二个实现**。MCP server 零配置、`LoopHook` 零第二用户，
**不是判据不成立，是还没有第一个实现。***

- [x] **50.1** ✅ 2026-09-16 —— **又收窄了一次：做答案，不做机械**
    - [x] 原文是「把三类资产的安装、启用、禁用、版本收成同一套声明式装配」。
          用三问重过一遍后先问：**这套机制是为了回答什么？**
          答案是「现在有什么在扩展这个 agent，各自从哪来，什么状态」。
          注册表 / 生命周期 / 热插拔都是**为回答它而存在的机械**，
          而在单人 CLI 上机械的成本高于答案的价值。
    - [x] `harness_inspect{what:"extensions"}` 一张表覆盖 skill / mcp / task。
          真实安装上验证：11 个 skill、1 个 task，`doc_parser` v2、`vision` v1。
    - [x] **`state` 是一句话不是布尔值**——三类资产的「启用」根本不是同一件事。
          skill 激活前什么都不做（静息态是 *installed* 而非 *enabled*）；
          MCP 的 `enabled = true` 说的是配置，**它是否真连上了是另一回事**
          （分开渲染，否则这一行描述的是配置而不是世界）；
          task 归 launchd，**本进程无从确认它有没有被调度**。
          压成统一 lifecycle 需要这些差异不成立。
    - [x] **兑现 44.2**：`Skill` 新增 `version` / `source`。闸门能拒绝破坏冒烟集的
          skill，但「回滚到能用的那个版本」要先知道当时跑的是哪个——
          在写下来之前，回滚是靠记性完成的。
- [x] **50.2** ✅ 一次 Run 用一份快照**已经是现状**（激活的 skill 持有在 `App` 上，
      MCP 启动时连接），把它形式化成机制不改变任何行为。
      **改为在列表里说出来**：「不热插拔，改动下一次 Run 生效」——
      一个不说明自己不做什么的清单，会被读成它做了。

**明确不做**（写进 P3 表）：任意 Rust 动态库加载 / 热卸载（判据未变）；
**Profile / 工作模式框架**（Skill + TASK.md + 项目级 `.seekcli.toml` 深合并已覆盖）；
`RunObserver` / `ContextContributor` / `RunGuard` 三类扩展点
（`LoopHook` 零第二用户，叠三个抽象只是把这个事实重复三遍）。

---

### 本轮需同步修订的既有约束

按 [design-principles §6](docs/architecture/design-principles.md#6-变更本文件)，
改约束要写明它排除了什么以及触发的阶段号。

| 约束 | 改法 | 触发阶段 |
| --- | --- | --- |
| ~~「扩展需求由 MCP 承担」~~ | ✅ 已收窄为「进程外扩展由 MCP，带脚本的方法论扩展由 Skill；需 harness 理解其结果的做成内置 Tool」 | 43 |
| ~~「跨会话语义记忆」排除~~ | ✅ **已推翻**（design-principles §2.1）。收窄为「不做向量检索 / 嵌入语义记忆」 | 45 |
| 「插件框架 / profile / bundle」排除 | 收窄为「不做动态代码加载与 Profile 组合；声明式装配可做」 | 50 |
| 「在线自演化 Skill」排除 | 明确为「不允许未经对照评估与人工批准的自我修改」 | 49 |
| L8 Loop 定义 | 现在只是定时触发器，补上「改进闭环」这层含义 | 49 |

---

## 🔵 已知仍开放（未排期，但不是「不做」）

推进完阶段二十 ~ 三十三后核对层文档与路线图发现四项仍然开放；
第二轮调研又把 L4-7 加了进来（理由见该行）。
它们既没做完、也没有被判定为「明确不做」——列在这里而不是悄悄留在层文档里，
是因为 `scripts/check-gap-coverage.py` 现在会强制两边一致。

| 缺口 | 现状 | 为什么没做 |
| --- | --- | --- |
| L1-1 | 主循环 524 → 339 行，但仍无正式扩展点 | `LoopHook` 需要第二个用户才立得住，见 31.2 |
| L1-2 | 无 turn / step 模型，只有 `iter` | 阶段三十一只做了拆解。引入 turn/step 要动 `LoopResult` 与事件模型，收益目前只有「概念更清晰」 |
| L1-4 | 中断后**上下文**可恢复，**任务**不会自动接着跑 | `/resume` 还原对话已够用；要真正接着跑需在投影里识别末尾 `Interrupted` 并合成继续指令，是个独立小改动 |
| L5-2 | 子代理一次性，无法续跑 / 通信 | 30.4 已主动推迟并写明理由；完整的可续子代理还需要 mailbox 与生命周期管理 |
| L4-7 | 会话存储与 JSONL 编解码直接耦合，无法换后端 | 阶段三十六原为此立项，但立项理由（「39 是第一个真实用户」）在调研后被推翻——trajectory 导出是导出格式不是存储后端。SQLite 索引与跨会话检索都没排期，**所以现在没有第二个实现**。出现真实第二后端时再做，判据同 L1-1 |

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
