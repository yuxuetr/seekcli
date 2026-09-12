# 三评：自进化地基基线

> 评估时间：2026-09-12
> 被评估方：SeekCLI `main` @ `f0d3784`
> 基线：[2026-08-24 复评](2026-08-24-reassessment.md)
> **坐标系：D. 自进化地基口径（本次新增，见 [scoring-rubric.md §2](scoring-rubric.md#2-四个正交口径)）**

---

## 0. 结论

| 口径 | 08-24 复评 | 本次 | 说明 |
| --- | --- | --- | --- |
| A. 纠偏机制 | 95% | 95% | 未动代码 |
| B. 组件全景 | 80% | 80% | 未动代码 |
| C. 产品形态 | 70% | 70% | crates.io 已定为不做；多平台 release 的 tag 仍待打 |
| **D. 自进化地基（新）** | — | **约 40%** | 本次唯一新结论 |

**D 口径低不是退步，是换了一把尺子。** B 口径问「对照 dsh 的组件清单缺哪些框」，
答不了「这个系统能不能自己长出新能力」。两者正交：一个满分的 B 完全可能是 D 的零分
——一个组件齐全但全部编译期焊死的 harness 就是这样。

**核心判断**：seekcli 在「产生变异」的四条上弱（自描述几乎为零），
在「保留变异」的三条上**意外地强**，其中两条**强于 dsh**。
缺口集中在一处——**运行期的自描述**——而不是分散在各层。

---

## 1. 七条件模型（本次方法学产出）

一个智能体系统要能「自动进化」，需要七个条件。前四条回答**怎么产生变异**，
后三条回答**怎么保留正确的变异**。

### A 组：产生变异

| # | 条件 | 缺了会怎样 |
| --- | --- | --- |
| 1 | **自描述** —— 智能体能读到自己的运行时契约 | 自修改退化成穷举盲试 |
| 2 | **自修改** —— 能给自己加能力 | 只能等人改代码 |
| 3 | **可逆** —— 加上去的能拆干净 | 长会话累积孤儿注册，单向熵增 |
| 4 | **分档持久** —— 变异有临时/会话/用户/仓库的档位 | 实验性变异污染生产配置 |

### B 组：保留变异

| # | 条件 | 缺了会怎样 |
| --- | --- | --- |
| 5 | **可判定的评价信号** | 进化 = 随机游走。模型自称「改好了」不是信号 |
| 6 | **选择压力 + 闸门** | 变异全部保留，能力面无限膨胀 |
| 7 | **否决记账** | 下一轮把上一轮否决过的方案原样重提 |

> **第 5 条是分水岭。** 变异若无法被机器判定好坏，其余六条做得再好也只是「可编程」，不是「可进化」。

---

## 2. 逐条评分

| # | 条件 | dsh | SeekCLI | 证据 |
| --- | --- | --- | --- | --- |
| 1 | 自描述 | ✅ 生成式 API catalog + 4 个 inspect provider + CI 新鲜度门禁 | ❌ **模型看不到自己的运行时**，无任何 inspect 工具 | 见 §3 |
| 2 | 自修改 | ✅ 代码级，进程内即时生效 | ⚠️ prompt 级有（`src/tools/meta.rs` `create_skill`），代码级无 | MCP 加工具需改配置 + 重启 |
| 3 | 可逆 | ✅ effect 化，卸载等 quiescence | ➖ 编译期组合，不存在需要可逆的注册 | 非缺陷，是架构选择的结果 |
| 4 | 分档持久 | ✅ 临时/会话/用户/仓库四档 | 🟢 两档半：proposals → active skill、`TASK.md`、`events.jsonl` | `src/skills.rs:82` |
| 5 | 评价信号 | 🟡 有 benchmark，**不回灌** | 🟢 **Fail-to-Pass，含反向安全断言** | `src/observability/bench.rs` |
| 6 | 选择压力 | 🟡 人工 PR 评审，无机械闸门 | ✅ `/skill accept｜reject` 是真闸门 | `src/skills.rs:82` |
| 7 | 否决记账 | ✅ 1948 篇 Agent Note，四态生命周期 | ✅ 三桶模型 + `check-gap-coverage.py` **三向校验** | 完整性被机械强制 |

### 两处 SeekCLI 强于 dsh

**第 6 条（选择压力）**：dsh 的四档持久没有升档闸门——文档明写「保留一个实验意味着
请 Agent 走正常开发流程实现一个正式插件」，即完全交回人工。SeekCLI 的
`proposals/` → `accept`/`reject` 是一条**已经存在的机械闸门**，只是目前只有 skill 一种资产能走。

**第 7 条（否决记账）**：dsh 的 `rejected/` 是**逐提案**的否决记录，没有任何东西校验
「边界清单是完整的」。SeekCLI 的 `check-gap-coverage.py` 强制每个缺口编号恰好落入
三桶之一，**对「我们明确不做什么」的完整性做机械校验**。dsh 无等价物。

---

## 3. 新缺口定义

> 依 [scoring-rubric §3](scoring-rubric.md#3-分层评分标准) 的约束，每条都附具体证据。

| 缺口 | 层 | 内容 | 证据 | 级别 |
| --- | --- | --- | --- | --- |
| L1-6 | L1 | **拒绝路径不可行动**：策略门拒绝只给 reason，不给「当前允许什么 / 建议的替代调用」 | `src/tools/policy.rs` `Verdict::Deny(reason)`；对照做对了的样板 `src/tools/mod.rs:57` `execute_with` 的 `bad_args` 分支 | 功能级 |
| L7-6 | L7 | **无自描述**：模型无法查询自己的工具面、生效策略、skill、MCP 状态 | 全 crate 无 inspect 类工具；`src/tools/registry.rs` 的工具面对模型只读不可查 | 功能级 |
| L4-7 | L4 | **会话存储无 seam**：`Session` 与 JSONL 编解码直接耦合，无法换后端 | `src/session.rs:342` `to_jsonl` / `:355` `from_jsonl` 是自由函数；全 crate 仅 2 个 trait（`src/api/mod.rs:230`、`src/api/tokens.rs:17`） | 功能级 |
| L5-6 | L5 | **提案闸门只服务 skill**：MCP 配置、策略规则、TASK.md 无法走同一道人工闸门 | `src/tools/meta.rs` `create_skill` 是唯一的提案入口 | 功能级 |
| L7-7 | L7 | **评价信号不回灌**：26 个 eval 任务与提案闸门之间无连接 | `src/observability/bench.rs` 与 `src/skills.rs:82` `accept_proposal` 无调用关系 | 功能级 |
| L7-8 | L7 | **trajectory 不可导出**：`events.jsonl` 是内部格式，外部流程无法消费 | `src/session.rs` 无导出路径；`--bench` 只产出聚合分数 | 功能级 |

六条全部排入阶段三十四 ~ 三十九，见 [TODOs.md](../../TODOs.md)。

---

## 4. dsh 的架构核心：读源码而非读文档的结论

本次首次读了 vendored 源码（`vendor/cordis/src/`，4.0.0-rc.7），结论与文档宣传的重点不同。

**代码分布本身就是答案**：

```
2693 行核心，其中
  fiber.ts     754 行 (28%)   ← 最大单文件
  reflect.ts   418 行 (16%)
  context.ts   146 行          ← 「上下文」本身只有 146 行
```

> **Cordis 的核心不是「插件」或「服务注册表」，是 `Fiber`——一个带可逆副作用的生命周期状态机。**

三个机制：

1. **Fiber 六态状态机**（`fiber.ts:147`）：`PENDING / LOADING / ACTIVE / FAILED / UNLOADING / DISPOSED`。
   关键在 `PENDING`——依赖不满足时是**挂起等待**而非失败；服务消失则退回 PENDING 并解绕注册。
   **`inject` 不是「注入」，是订阅**。这条把 Cordis 与一次性解析依赖图的普通 DI 分开。
2. **Effect 可逆，难点全在重入**。dsh 自己 fork 后最大的一笔改动就是补这里
   （`vendor/README.md` 本地修改日志第 6 条，整段讲 reentrant disposal 的三个洞），
   代码层面对应 `fiber.ts:420`：`UNLOADING` 状态下拒绝创建 effect，防止清理期注册逃出卸载快照。
3. **Context 是 Proxy**（`reflect.ts:135`）。由此白拿 `isolate` 作用域、`intercept` 配置合并，
   以及**每个插件持有自己的 ctx**——归属信息是可逆性的前提。

### 为什么 SeekCLI 不抄

> **Cordis 的 Fiber 本质上是在 GC 语言里手工重建 RAII + 作用域生命周期。Rust 编译器免费提供这个。**

`Drop` + 所有权 + 作用域就是 effect 可逆性的编译期版本。754 行状态机模拟的东西，
Rust 是语言内置不变量；Cordis 需要 `INACTIVE_EFFECT` 运行时检查拦截的错误，Rust 里编不过。

三条核心机制的对位：

| Cordis 机制 | SeekCLI 的答案 |
| --- | --- |
| Effect 可逆 | **已天然拥有**（`Drop` / RAII），零成本 |
| 响应式依赖（PENDING 语义） | 能做，但需真有第二个实现才值得（判据同 L1-1） |
| 运行期加载新代码 | **Rust 拿不到**；正确答案是进程边界（MCP） |

### 一处可判定的越线

dsh 自身文档提供的证据：`sdk-minimal` 是 "deliberate exception"——一个 bundle
拥有完整显式 SDK 树且不 apply `dsh-base`；以及 `verify-application-entrypoints.ts`
的存在，它要「拒绝任何绕过 `dsh` 的 Node 应用路径」。

> 当你需要一个 CI 脚本来强制「所有入口必须走同一个启动器」时，说明无内核架构产生了
> 太多可能的入口，于是不得不重新发明一个内核——只是用 CI 强制，而非类型系统。

同一模式见于 waterfall：dsh 的 Agent Note 明写「一个 mount 的 `tools/pre-execute`
listener 不调 `next()` 就能停掉 agent 自己的工具派发」。
而 `src/tools/mod.rs:57` `execute_with` 的单槽形态，用类型系统拿到了
**「没有任何工具能绕过策略门」的硬保证**——在这个具体不变量上 SeekCLI 严格更好。
中间件链会引入短路 gate 的可能，**这是取舍不是欠账**（记入 P3）。

---

## 5. 关于强化学习

**dsh 完全不涉及 RL。** 全仓 grep `reinforcement|reward|GRPO|RL|hermes` 零命中——
它是 harness，不训练任何东西。Hermes Agent 的自演化则是
「拆到独立仓库离线优化」（见 [架构总纲 §4](../architecture/README.md#4-与-hermes-agent-的对位)）。

进化有四个层级，只有最后一级需要 RL：

| 层级 | 改什么 | 延迟 | 需要 RL |
| --- | --- | --- | --- |
| L-a 上下文内 | reminder、压缩、**教学式错误** | 毫秒 | 否 |
| L-b 会话内 | 挂工具 / 服务 / 监听器 | 秒 | 否 |
| L-c 跨会话 | 持久化的 skill / 插件 / 策略 | 天（人工闸门） | 否 |
| L-d 权重级 | 模型参数 | 周（离线） | **是** |

> **harness 不是 RL 的替代品，是 RL 的前置基础设施。**

RL 需要三样东西，而 SeekCLI 已各有雏形：

| RL 要的 | SeekCLI 的对应物 | 状态 |
| --- | --- | --- |
| 可重置环境 | `src/observability/bench.rs` 每任务起隔离 testbed | ✅ |
| 可判定 reward | Fail-to-Pass 退出码，含反向断言 | ✅ 26 任务 |
| 可回放 trajectory | `events.jsonl` + LLM 录制/回放 | 🟡 格式内部，见 L7-8 |

**因此本次给出一条定位补充**（不改主定位）：

> SeekCLI 是 RL / 进化实验的**环境与评测器**，不是训练器。

理由：完全复用已有 L7 基建，不引入新架构概念，训练端留在外部——
不违反 [design-principles §2](../architecture/design-principles.md#2-明确排除的复杂度) 任何一条。

---

## 6. 一处可以领先 dsh 的地方

**把 eval 接进提案闸门**（L7-7）。

dsh 做不到：它的 e2e / snapshot / web 测试在 CI 里跑，不在会话里跑。
单人本地 CLI 的 eval **可以在接受提案的那一刻当场跑完**——
体量劣势反过来变成结构优势。这是「自扩展」升级成「进化飞轮」缺的那一节。

---

## 7. 该学与不该学

| 学（三条，全不依赖插件框架） | 不学 |
| --- | --- |
| **seam 三角色纪律**——加能力时同时设计定义/实现/消费。`LlmProvider` 已做对一次（四个装饰器实现） | Cordis 式 DI / Proxy context |
| **否决记账**——尤其 `## Alternatives considered` 强制章节 | profile / bundle 组合 |
| **教学式错误**——把知识写进拒绝路径而非 system prompt | 运行期代码挂载（dlopen / WASM） |

判断何时该改主意的判据不变：**当某一层出现真实的第二个实现时**。
L0 已出现（Resilient / Recording / CostTracker / Replay 四个）故已解锁；
L4 会话存储是下一个（L7-8 trajectory 导出、SQLite 索引、跨会话检索，任一个都够）。

---

## 8. 方法学说明

- dsh 侧本次**首次读了 vendored 源码**（`vendor/cordis/src/` 全部、`vendor/README.md`
  本地修改日志、`packages/extensions/` 全组、`.agents/notes/` 格式规范），
  不再只依赖其自述文档；行号与代码分布数据均来自实读。
- SeekCLI 侧来自全量源码走查；本次**未跑** `cargo test` / `llvm-cov`——
  D 口径评的是架构能力而非实现质量，A/B/C 三口径分数沿用复评结论未重新验证。
- 七条件模型是本次新增的坐标系，**不与 A/B/C 的历史数字比较**。
