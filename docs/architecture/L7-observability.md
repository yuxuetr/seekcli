# L7 可观测层：Cost · Tracing · Benchmark

> 完成度 **95%**（阶段二十五、二十九后）｜ 缺口来源：[评估 §3 L7](../evaluation/2026-08-harness-gap-analysis.md#l7-可观测层--55)

## 1. 职责边界

让「引擎是否变好」可量化。**没有这一层，前面所有改动都是凭感觉。**

三个问题：花了多少（Cost）、怎么走到这一步的（Tracing）、改完之后是变好还是变坏（Benchmark）。

## 2. 当前实现

| 模块 | 位置 | 说明 |
| --- | --- | --- |
| Cost Tracker | `observability/cost.rs` | 装饰器式累加 prompt/completion/cache token + 调用数；`estimated_cny` + `cache_hit_pct`；随 session 持久化 |
| Tracing | `observability/trace.rs` | Run → Turn → Generate/Execute/Planning/Compaction span 树 → `~/.seekcli/traces/<run_id>.json`；`SEEKCLI_TRACE` 开关，关闭时零成本 no-op |
| 录制 / 回放 | `api/record.rs` | `SEEKCLI_RECORD` / `SEEKCLI_REPLAY`，fixture 在 `tests/fixtures/` |
| Benchmark | `observability/bench.rs` | Testsuite JSON → seed 靶机 → chdir 沙箱 AgentRun → eval 命令 → 报表（成功率 / CNY / 耗时 / 调用数） |

**装饰器 / 旁路埋点**的做法是对的，`run_agent_loop` 里没有混入计费代码，**保持**。

## 3. 缺口

| # | 缺口 | 证据 | 影响 |
| --- | --- | --- | --- |
| ~~L7-1~~ | ~~eval 数据集只有 3 个任务~~ | **阶段二十九已落地**：6 套件 26 任务 | — |
| ~~L7-2~~ | ~~agent 循环本身零自动化测试~~ | **阶段二十五已落地**：`engine.rs` 行覆盖率 13.8% → **65.0%**，总体 46.3% → **65.6%** | — |
| L7-3 | 无快照回归 | 无 | 提示词 / 流程变更无差异可看 |
| ~~L7-4~~ | ~~覆盖率门禁形同虚设~~ | **阶段二十 20.5 已落地**，棘轮 45 → 60 | — |
| L7-5 | 无 OTel 导出 | 取舍级 | |
| ⚠️ L7-6 | 无自描述：模型看不到自己的运行时 | 全 crate 无 inspect 类工具；工具面、生效策略、skill、MCP 状态对模型均不可查 | 被拒后只能盲猜原因 |
| L7-7 | 评价信号不回灌 | `bench.rs` 与 `skills.rs::accept_proposal` 无调用关系 | 进化无选择压力 |
| L7-8 | trajectory 不可导出 | `--bench` 只产出聚合分数 | 外部 RL / 进化流程无法消费 |

> L7-2 是 L1 / L2 / L4 三处重构的**前置条件**。
> 没有可回放的测试，把 440 行主循环拆成四个阶段函数是在裸奔。

## 4. 目标设计

### 4.1 LLM 录制 / 回放（L7-2）✅ 阶段二十五已落地

```
SEEKCLI_RECORD=fixtures/xxx    每次 API 响应流按顺序落盘
SEEKCLI_REPLAY=fixtures/xxx    从盘里按顺序喂回，完全不发网络请求
```

实现：一个包住 `LlmProvider` 的装饰器（与 `Resilient` / `CostTracker` 同一手法）：

```rust
pub struct Recording<P> { inner: P, dir: PathBuf }
pub struct Replaying { dir: PathBuf, cursor: AtomicUsize }
```

- 录制格式：`<seq>.jsonl`，一行一个 `StreamItem`，另存一份 `request.json` 便于人读。
- 回放时**校验请求形状**（消息条数、最后一条角色、工具集哈希）；
  不匹配就报错而不是静默错位——这才是它作为回归测试的价值所在。
- 有了它，`run_agent_loop` 进了 `#[tokio::test]`：录了真实的
  「读不存在的文件 → 失败 → Two-Stage → 创建」轨迹，
  已验证故意破坏 `append_plan_with_bridge` 会让该测试失败——
  **阶段十九那个 bug 不会再静默回归**。
- 形状校验刻意只查**结构**（消息条数 / 最后一条角色 / 工具集），不查逐字内容：
  改措辞不该让整套 fixture 失效，但换一段对话必须失败。
- 副作用断言优先于文本断言：阶段十九 bug 的特征正是模型**声称**做完了，
  所以测的是「文件是否真的存在」而不是「回答里有没有说成功」。
- 测试踩到了三处进程级全局状态（cwd、`policy::MODE`、`approval::INTERACTION`）
  与 `cargo test` 并行执行的冲突，用 `crate::testsync` 单锁串行化——
  否则一个测试翻转策略模式会让无关测试失败，看起来像被测代码的 bug。

### 4.2 eval 套件扩容（L7-1）✅ 阶段二十九已落地

`examples/benchmarks/` 从 1 个文件扩到按能力分组，目标 20+ 任务：

| 套件 | 覆盖 | 任务数目标 |
| --- | --- | --- |
| `fs.json` | 读 / 写 / 编辑 / 检索 | 6 |
| `shell.json` | 命令执行、失败恢复、退出码理解 | 4 |
| `multistep.json` | 需要 3+ 步与工具组合的任务 | 5 |
| `recovery.json` | 故意制造失败，考察 Recovery / Two-Stage | 4 |
| `safety.json` | 危险命令、越权写、Plan Mode 门禁 | 4 |

`safety.json` 的 eval 断言是**反向**的：期望模型**没有**做成某件事。
`expect_fail: true` 让 eval 命令保持为「陈述句」——写成取反的 shell 表达式会把
意图埋进 `!` 和 `test` 里。配套的 `flags`（如 `--read-only`）把「在哪个模式下测」
留在套件内，而不是取决于运行器当时怎么被调用——一个没有 flags 的反向断言会在
普通模式下悄悄通过，所以有单测强制两者同时出现。

### 4.3 CI 门禁（L7-4）✅ 阶段二十已落地

`build.yml` 补上实际调用：

```yaml
- run: >-
    cargo llvm-cov nextest --all-features --workspace
    --summary-only --fail-under-lines 45
```

一条命令同时跑测试与测覆盖率，门禁不会与实际跑的测试脱节。

阈值取 **45**：2026-08-22 采纳时实测行覆盖率为 46.29%，取略低于基线的值作为**棘轮**。
**不追求高覆盖率数字**，目的是防止「新增模块零测试」这类退化——
随重构上调，**永远不为了让 CI 变绿而下调**。

### 4.4 Cost / Trace 小改 ✅ 阶段二十九已落地

- Cost：`estimated_cny` 的费率移进 `config.toml`（现在硬编码），并标注「估算」。
- Trace：span 树增加 `verdict`（acted / answered / empty）、`content_bytes` 与本轮工具名单，
  让「模型声称完成但该轮 tool_calls=0」一眼可见——阶段十九是靠人肉读 trace 发现的，
  命名之后下一次可以直接 grep。
- ⚠️ 顺带修掉一个真实缺陷：**headless 下 tracing 完全不工作**。`run_headless` 从不调用
  `start_run` / `flush`，于是 `-p` / `--bench` / `--run-task` 一条 trace 都不产生——
  而那恰恰是没人盯着终端、最需要 trace 的模式。

### 4.5 自描述 `harness_inspect`（L7-6）

> **原则**：自描述的收益先于自修改兑现。dsh 的 Agent Note 点明过代价——
> 模型猜方法签名、猜返回值形状要花很多步盲试。即使永远不做自修改，这条也值。

#### 4.5.1 一个只读工具，五个分区

| `what` | 内容 | 数据来源 |
| --- | --- | --- |
| `tools` | 当前工具面：名字 + 调用签名 + 来源（内置 / MCP / skill） | 循环里的 `effective_tools` |
| `policy` | 当前 mode + **生效中的**只读命令表、会变更状态的子命令表、无条件拒绝类别 | `tools/policy.rs` 的常量本身 |
| `skills` | 活跃 skill 与待审提案 | `SkillManager` |
| `mcp` | 各 server 状态与**失败原因** | 34.2 落地的 `McpRegistry.failures` |
| `session` | 事件数、压缩次数、token 估算 | 会话事件日志 |

#### 4.5.2 不得成为第二份会漂移的事实来源

`policy` 分区必须从 `policy.rs` 的**同一份常量**渲染，不得另写一段描述。
手写描述会漂移，而漂移时模型信的是错的那一份——
那比不提供自描述更糟，和 §4.6.6 建议下限是同一条道理。

为此 `policy.rs` 暴露只读访问器而不是把常量复制一份。

#### 4.5.3 为什么走 `execute_with` 而不是引擎分支

`invoke_agent` / `load_skill` 绕过 `ToolDispatcher`，理由是前者重入循环、
后者改引擎状态（[L1 §4.1](L1-engine.md)）。**`harness_inspect` 不属于这一类**：
它只读，没有重入，没有状态变更。

因此它走 `execute_with`——和 MCP 工具同一条路——保住「没有任何工具能绕过
策略门、deadline 与审计」这个不变量。代价是引擎要先把它需要的 App 状态
装成一个纯数据 `Snapshot` 再交给渲染函数。

这个分层顺带让渲染**纯函数可测**，与 `bench.rs`「owns the pure, testable pieces」
同一手法：不需要构造 `App` 就能断言输出。

#### 4.5.4 验收

- 模型在 `--read-only` 下被拒一次后，能用 `harness_inspect{what:"policy"}`
  自行查出哪些命令可用，而不是盲试。
- `policy` 分区的内容与 `policy.rs` 常量同源——改常量则输出随之变化，有单测守。
- 归入 `is_parallel_readonly`：它是纯读，没有理由阻塞并发批次。

## 5. 明确不做

| 项 | 理由 |
| --- | --- |
| OTel 导出 | 单机 CLI 没有 collector 可送；JSON span 树 + `jq` 已够 |
| 快照测试框架 | 录制回放已覆盖主要回归需求，再加一套是重复投资 |

## 6. 验收标准

- `SEEKCLI_REPLAY=...` 下 `cargo test` 能跑通一条完整的多步 agent 轨迹，全程零网络。
- 故意改坏 `append_plan_with_bridge` → 回放测试失败（证明它真的在守护阶段十九的修复）。
- `cargo llvm-cov` 在 CI 中实际执行且阈值生效。
- `seekcli --bench examples/benchmarks/recovery.json` 给出成功率与成本报表。

## 7. 对应路线

**阶段二十五**（LLM 录制/回放——因是三次结构性重构的前置条件，单独提前）、阶段二十九（eval 套件扩容 + CI 覆盖率门禁，P2）。
