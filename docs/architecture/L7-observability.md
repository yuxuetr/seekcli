# L7 可观测层：Cost · Tracing · Benchmark

> 完成度 **55%** ｜ 缺口来源：[评估 §3 L7](../evaluation/2026-08-harness-gap-analysis.md#l7-可观测层--55)

## 1. 职责边界

让「引擎是否变好」可量化。**没有这一层，前面所有改动都是凭感觉。**

三个问题：花了多少（Cost）、怎么走到这一步的（Tracing）、改完之后是变好还是变坏（Benchmark）。

## 2. 当前实现

| 模块 | 位置 | 说明 |
| --- | --- | --- |
| Cost Tracker | `observability/cost.rs` | 装饰器式累加 prompt/completion/cache token + 调用数；`estimated_cny` + `cache_hit_pct`；随 session 持久化 |
| Tracing | `observability/trace.rs` | Run → Turn → Generate/Execute/Planning/Compaction span 树 → `~/.seekcli/traces/<run_id>.json`；`SEEKCLI_TRACE` 开关，关闭时零成本 no-op |
| Benchmark | `observability/bench.rs` | Testsuite JSON → seed 靶机 → chdir 沙箱 AgentRun → eval 命令 → 报表（成功率 / CNY / 耗时 / 调用数） |

**装饰器 / 旁路埋点**的做法是对的，`run_agent_loop` 里没有混入计费代码，**保持**。

## 3. 缺口

| # | 缺口 | 证据 | 影响 |
| --- | --- | --- | --- |
| L7-1 | **eval 数据集只有 1 个文件 3 个任务** | `examples/benchmarks/basic.json` | 有跑道没有车，回归检测力接近零 |
| L7-2 | **agent 循环本身零自动化测试** | 87 单测全是纯逻辑 | 主控流任何重构只能靠手测 |
| L7-3 | 无快照回归 | 无 | 提示词 / 流程变更无差异可看 |
| L7-4 | 覆盖率门禁形同虚设 | `build.yml` 装了 `cargo-llvm-cov` 却从未调用 | CI 里那一步是死代码 |
| L7-5 | 无 OTel 导出 | 取舍级 | |

> L7-2 是 L1 / L2 / L4 三处重构的**前置条件**。
> 没有可回放的测试，把 440 行主循环拆成四个阶段函数是在裸奔。

## 4. 目标设计

### 4.1 LLM 录制 / 回放（L7-2，**优先级最高**）

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
- 有了它，`run_agent_loop` 可以进 `#[tokio::test]`：
  录一次真实的「读不存在的文件 → 失败 → Two-Stage → 创建」轨迹，
  之后每次重构都跑一遍，**阶段十九那个 bug 将永远不会静默回归**。

### 4.2 eval 套件扩容（L7-1）

`examples/benchmarks/` 从 1 个文件扩到按能力分组，目标 20+ 任务：

| 套件 | 覆盖 | 任务数目标 |
| --- | --- | --- |
| `fs.json` | 读 / 写 / 编辑 / 检索 | 6 |
| `shell.json` | 命令执行、失败恢复、退出码理解 | 4 |
| `multistep.json` | 需要 3+ 步与工具组合的任务 | 5 |
| `recovery.json` | 故意制造失败，考察 Recovery / Two-Stage | 4 |
| `safety.json` | 危险命令、越权写、Plan Mode 门禁 | 4 |

`safety.json` 的 eval 断言是**反向**的：期望模型**没有**做成某件事。
这类用例在现有 Fail-to-Pass 范式下要显式支持 `expect_fail: true`。

### 4.3 CI 门禁（L7-4）

`build.yml` 补上实际调用：

```yaml
- run: cargo llvm-cov --all-features --workspace --lcov --output-path lcov.info
- run: cargo llvm-cov report --fail-under-lines 60
```

阈值从 60 起步，随重构逐步上调；**不追求高覆盖率数字**，
目的是防止「新增模块零测试」这一类退化。

### 4.4 Cost / Trace 小改

- Cost：`estimated_cny` 的费率移进 `config.toml`（现在硬编码），并标注「估算」。
- Trace：span 树增加 `tool_calls` 计数与 `verdict` 字段，
  让「模型声称完成但该轮 tool_calls=0」这类问题一眼可见（阶段十九靠人肉发现的）。

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
