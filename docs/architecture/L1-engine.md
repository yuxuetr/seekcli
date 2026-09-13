# L1 引擎层：ReAct 主循环与运行时纠偏

> 完成度 **90%**（阶段三十一后）｜ 缺口来源：[评估 §3 L1](../evaluation/2026-08-harness-gap-analysis.md#l1-引擎层--80最强的一层)

## 1. 职责边界

驱动 think → act → observe 闭环，并在模型跑偏时**运行时纠偏**。

「Harness 区别于裸 ReAct」的分界线就在本层：裸 ReAct 只有循环，
Harness 还有 Two-Stage、Reminders、Recovery、并发编排、迭代上限。

## 2. 当前实现

| 机制 | 位置 | 说明 |
| --- | --- | --- |
| ReAct 主循环 | `engine.rs::run_agent_loop` | `for iter in 0..max_iter` |
| Two-Stage ReAct | `engine.rs::planning_phase` | 宏触发（首轮 + thinking 开）/ 微触发（上轮工具失败） |
| 规划轮护栏 | `append_plan_with_bridge` / `planning_only_directive` / `strip_fake_tool_syntax` | 阶段十九根因修复 |
| System Reminders | `agent/reminders.rs` | 连续 3 次相同轨迹注入 user 消息打断 |
| Error Recovery | `agent/recovery.rs` | 按工具 + 错误类型追加 `[Recovery]` 建议 |
| Fork-Join 并发 | `engine.rs` + `registry::is_parallel_readonly` | 批次全只读才并发 |
| 迭代 / 深度上限 | `agent/mod.rs::MAX_ITER` / `MAX_SUBAGENT_DEPTH` | 25 / 3 |
| 中断 | `main.rs::spawn_interrupt_watcher` + 循环内轮询 | Ctrl-C 优雅回 REPL |

> 阶段十九对「假工具调用」的根因分析值得保留为范本：
> 问题不在 Rust 解析，而在**消息序列形状分布外**（规划轮后以 assistant 结尾）
> 与**系统提示自相矛盾**（无工具时仍要求「别叙述、直接调用」）。
> 修复 = 恢复 assistant→user→assistant 交替 + 临时指令 + 确定性兜底截断。

## 3. 缺口

| # | 缺口 | 证据 | 性质 |
| --- | --- | --- | --- |
| ⚠️ L1-1 | 循环仍无正式扩展点 | 主循环 524 → **339 行**（阶段三十一拆解）；`LoopHook` 未做，理由见 §4.2 | 结构 |
| ⚠️ L1-2 | 无 turn / step 概念 | 只有 `iter`。阶段三十一只做了拆解，未引入 turn/step 模型 | 结构，**未排期** |
| ~~L1-3~~ | ~~无运行中上下文注入~~ | **阶段三十已落地**：后台任务完成通知在 step 顶部注入 | — |
| L1-4 | 中断即终止，无法续跑 | Ctrl-C 后直接结束 | 功能 |
| ~~L1-5~~ | ~~取消不向下传播~~ | **阶段三十一已落地**，实测 SIGINT 后子进程从 2 个降到 0 | — |
| ⚠️ L1-6 | 拒绝路径不可行动 | `policy.rs::check_with` 的 mode 门对 `run_shell` 给出**与事实不符**的消息；命令门丢掉了「哪个子命令定的罪」 | 功能 |

## 4. 目标设计

### 4.1 拆解主循环（L1-1 / L1-2）✅ 阶段三十一已落地（部分）

把 440 行拆成四个阶段函数，主循环只做编排：

```rust
struct StepCtx<'a> {
  messages: &'a mut Vec<Message>,
  depth: usize,
  iter: usize,
  cancel: &'a CancelToken,
}

enum StepOutcome {
  Continue,            // 有工具结果，进入下一 step
  Done(String),        // 模型给出最终答复
  Interrupted,
  LimitReached,
}
```

实际抽出的是**自成一体**的两块，而不是设计稿里那四个阶段函数：

- `request_step` —— 把一条 delta 流变成一个响应。它没有自己的控制流，
  渲染、usage 记账、流内中断检查都属于这件事而非循环。
- `delegate_to_subagent` / `activate_skill_by_name` —— 引擎级的委派工具。
  它们不走 dispatcher（前者会重入循环，而 dispatcher 刻意不认识循环），
  在主循环里只剩两行调用。

**524 → 339 行，行为零变化**，由阶段二十五的回放测试守住，并做了子代理委派的真实回归。

未按设计稿切成 `prepare_step` / `observe`：剩下的部分不是「可以搬走的整块」，
而是循环自身的控制流（迭代上限、中断、plan_next、reminder 触发）。
硬切只会把控制流分散到多个函数、靠参数传状态，读起来更难而不是更容易。

### 4.2 LoopHook 扩展点（L1-1）—— 暂不做

```rust
#[async_trait]
pub trait LoopHook: Send + Sync {
  async fn pre_step(&self, _cx: &mut StepCtx<'_>) -> Result<Control> { Ok(Control::Proceed) }
  async fn post_step(&self, _cx: &mut StepCtx<'_>, _out: &StepOutcome) -> Result<()> { Ok(()) }
}

pub enum Control { Proceed, SkipStep, StopTurn(String) }
```

**暂不做**，理由与阶段二十七不做 `ToolImpl` 相同：抽象要有第二个用户才立得住。

现在挂在循环上的机制是 compressor、reminders、tracer、job 完成通知、recovery，
全部是内部的、编译期已知的、且各自已有单测。把它们套进 `Vec<Box<dyn LoopHook>>`
换来的是一层间接，而不是任何新能力——**没有第三方插件要挂进来**。

真正让它值得做的信号是：出现一个需要在循环里插手、但又不该进 `engine.rs` 的东西。
MCP 没有产生这个需求（它挂在工具层），L8 也没有（它在循环外）。
届时再引入，那时它才有真实用户。

### 4.3 取消传播（L1-5）

`Arc<AtomicBool>` 升级为携带 `tokio::sync::Notify` 的 `CancelToken`，
经 `StepCtx` 传到 `tools::shell::run_shell`，用 `tokio::select!` 在
子进程 `wait()` 与取消之间竞争，取消时 `child.start_kill()`。

### 4.4 可续跑（L1-4）—— 部分落地

阶段二十六已做：中断写入 `Interrupted` 事件，`/resume <id>` 从日志重建对话。
**未做**：模型不会自动从断点继续——恢复的是上下文，不是任务。
要真正「接着跑」，需要在投影里识别末尾的 `Interrupted` 并合成一条继续指令，
这是个独立的小改动，尚未排期。

### 4.5 上下文注入（L1-3）

```rust
impl App { pub fn inject(&mut self, note: String) }  // 挂到下一次 prepare_step
```

用途：后台 job 完成通知（L2）、L8 digest 提醒、文件变更。

### 4.6 教学式错误（L1-6）

> **原则**：把知识写进**拒绝路径本身**，而不是写进 system prompt。
> 前者按需出现、精确指向这一次的上下文；后者每轮都付 token 且容易被忽略。

#### 4.6.1 为什么必须改 `policy.rs` 而不是 `recovery.rs`

`ToolKind::Denied.is_failure()` **刻意**返回 `false`——
「拒绝是一个决定，不是一次故障；把它当故障会让模型绕着策略重新规划，
而策略的存在就是为了阻止这件事」（`tools/result.rs`）。

因此 Error Recovery 对拒绝**不会触发**，`recovery.rs::hint_for` 还额外把
`[USER DENIED]` / `[PATH DENIED]` 显式排除。结论：

> **拒绝的教学内容只能长在拒绝文本里。** 这是本节改 `policy.rs` 而不是
> 扩 `recovery.rs` 的结构性原因，不是实现偏好。

两者的职责分界必须保持：**`recovery.rs` 管故障，`policy.rs` 管拒绝，互不复制。**

#### 4.6.2 已经做对的，不要重做

| 路径 | 现状 | 样板价值 |
| --- | --- | --- |
| 路径门 | `path_security::ensure_within_cwd` 已给出工作区根 + 解析后路径 + 下一步建议 | **本节的参照标准** |
| 参数错误 | `tools/mod.rs::execute_with` 的 `bad_args` 分支，注释写明「Surface it explicitly so Error Recovery can hand the model an actionable hint」 | 同上 |
| 工具故障 | `recovery.rs` 按工具 + 错误形状给 `[Recovery]` SOP | 职责已分清 |

#### 4.6.3 mode 门的消息与事实不符（最高价值的一条）

`check_with` 第 1 步对任何被拒的 mutating 工具统一回：

```text
`run_shell` is not available in read-only mode.
```

但 `run_shell` 在 read-only 下**是可用的**——同一个测试既断言
`run_shell{command:"rm f"}` → Deny，又断言 `run_shell{command:"git status"}` → Allow。

> **模型被告知「这个工具没了」，而事实是「这一条命令不行」。**
> 它会整体放弃 shell，而不是换一条报告型命令——这是消息层面的正确性缺陷，
> 不只是措辞不够详细。

目标形态：区分两种拒绝。

```text
# 工具整体不可用（write_file / edit_file / create_skill）
`write_file` is not available in read-only mode. …do not retry this call.

# 工具可用但这条命令不是只读（run_shell）
`run_shell` is available in read-only mode, but only for reporting commands.
Refused because: `rm f` is not a read-only command.
Reporting commands remain available (ls / cat / grep / git status / …).
```

**约束**：判定逻辑零变化。本节只改拒绝消息的信息量——
判定与措辞必须分两步改，否则一次改动同时动了安全语义和文案，
回归时说不清是哪一边坏的。

#### 4.6.4 命令门丢掉了定罪的子命令

`classify_command` 在第一个 Deny 处返回并**丢弃是哪个 part**：

```rust
Decision::Deny(r) => return Decision::Deny(r),   // part 没有被带出来
```

`ls; rm -rf /` 被拒时模型只看到「recursive delete on system or home path」，
不知道是哪一段触发的——而它下一步最可能做的就是把整行重写一遍，
把真正的问题原样带回来。目标形态是把定罪的子命令带进理由，并区分
**Deny（无条件，审批也不能解）** 与 **Ask（人可以批准）**。

#### 4.6.5 `[MODE DENIED]` 不在系统提示的 do-not-retry 清单里

`agent/prompt.rs` 的 Safety 段只点了 `[USER DENIED]` 与 `[PATH DENIED]`，
而 `tools/mod.rs` 产出的第三种前缀 `[MODE DENIED]` 不在其中。
拒绝文本自己带了「do not retry」，但系统提示层面缺这一条。
**这是四处里唯一应当改 system prompt 的一处**——因为它说的是前缀的通用语义，
不是某一次拒绝的上下文。

#### 4.6.6 外来能力缺失不可见

`mcp/mod.rs` 的 server 启动失败是「跳过 + 一行可见告警」，符合
[设计原则 §4](design-principles.md#4-错误处理) 的「降级但绝不静默」——
但那行告警给的是**用户**，模型完全不知道少了哪些工具、为什么少。

**实现时修正了方向。** 初稿写的是「失败原因进上下文」，那等于加一段常驻
system prompt——与 §4.6.5「只有 `[MODE DENIED]` 那一处该改 system prompt」
自相矛盾，而且为一个多数会话里无关的事实每轮付 token。

改为：启动失败**记在 `McpRegistry` 上**，在模型真的去碰那个不存在的工具时
由拒绝路径讲出来。

```text
MCP server `github` is configured but unavailable this session, so
`mcp__github__create_issue` does not exist: <启动失败原因>.
Do not retry this tool. …
```

这样常驻 token 成本为零，且与本节「知识长在拒绝路径里」的原则一致。
同一份记录是[阶段三十五 `harness_inspect`](L7-observability.md) 的 `mcp` 分区
要读的数据——**按需查询**才是「少了哪些工具」这类问题的正确出口。

同理，未知工具名时应附**最接近的可用工具名**及其调用签名。建议必须有下限：
一个几乎不沾边的候选比不给建议更糟，它让模型带着虚假信心走错路。

#### 4.6.7 验收与两层断言的分界

实现时撞上一条限制，值得记下来：**eval 层看不到模型说了什么。**
`Task::run_eval` 只在 testbed 里跑一条 shell 命令看退出码，而 `HeadlessOutcome.text`
原先被丢弃。`--read-only` 下这一点是致命的——模型不被允许写任何东西，
于是 testbed 里根本不可能留下证据。

因此断言分两层，各管各的：

| 层 | 断言什么 | 为什么在这一层 |
| --- | --- | --- |
| **单测** | 消息**文本**含可行动信息（不得出现 "not available"、必须指名障碍、必须给出替代） | 确定性、零成本、不需要 API key |
| **eval** | 模型拿到消息后**行为**是否改变（换一条路并拿到结果） | 只有端到端才能证明消息真的管用 |

为让第二层可能，`run_eval` 把模型的最终回答以 `$SEEKCLI_ANSWER` 暴露给 eval 命令。
该文件**放在 testbed 之外**：断言「没有创建任何文件」的任务用 `ls -A`，
会把放进去的文件一起看见。这条通道同样是[阶段三十八 eval 回灌](L7-observability.md)需要的。

验收：

- 判定逻辑的既有单测全部不变——这是「只改措辞」的机械证据。
- `safety.json` 三条新任务：被拒后报出数值而非放弃、讲出障碍是什么、
  把理由类别（`privilege escalation`，**不在 prompt 里**）带回给用户。
- ⚠️ 三条 eval 任务需真实 LLM 调用，**尚未端到端跑过**（需要 API key 与费用）。

## 5. 验收标准

- 重构后 87 单测 + eval 套件全过，`SEEKCLI_TRACE` 决策树结构不变。
- Ctrl-C 时正在跑的 `sleep 60` 子进程在 1s 内消失（`ps` 可验证）。
- 新增一条循环策略只需实现 `LoopHook`，不改 `run_agent_loop`。

## 6. 对应路线

阶段三十一（循环拆解与扩展点，P2）、阶段二十六（可续跑随事件日志落地）。
