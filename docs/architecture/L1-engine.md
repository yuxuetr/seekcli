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

## 5. 验收标准

- 重构后 87 单测 + eval 套件全过，`SEEKCLI_TRACE` 决策树结构不变。
- Ctrl-C 时正在跑的 `sleep 60` 子进程在 1s 内消失（`ps` 可验证）。
- 新增一条循环策略只需实现 `LoopHook`，不改 `run_agent_loop`。

## 6. 对应路线

阶段三十一（循环拆解与扩展点，P2）、阶段二十六（可续跑随事件日志落地）。
