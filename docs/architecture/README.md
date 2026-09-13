# SeekCLI 架构总纲

> 本目录是 SeekCLI 的「宪法级」设计文档：定义心智模型、分层职责与每层的目标形态。
> 缺口从哪来见 [`docs/evaluation/`](../evaluation/README.md)；何时落地见根目录 [`TODOs.md`](../../TODOs.md)。

---

## 0. 项目定位

**SeekCLI = DeepSeek + Tools + Harness Agent 核心。**

一个**单纯的本地 CLI Agent**。不做多模态网关，不做内容平台胶水。
模型需要的能力应通过 **Tool Calling** 由 Agent 自取，而不是由 CLI 客户端预先注入
（历史上的 MinerU / StepFun VLM / Tavily / Jina / GLM Search 已于阶段七全部剥离）。

---

## 1. 八层心智模型

```
┌────────────────────────────────────────────────────┐
│  L8  循环层      系统级调度 · Headless Task · Digest │
├────────────────────────────────────────────────────┤
│  L7  可观测层    Cost Tracker · Tracing · Benchmark  │
├────────────────────────────────────────────────────┤
│  L6  界面层      REPL · CLI 子命令 · headless 入口   │
├────────────────────────────────────────────────────┤
│  L5  组合层      SubAgent 模板 · Skill 策展 · MCP    │
├────────────────────────────────────────────────────┤
│  L4  记忆层      会话事件日志 · 压缩 · 状态外部化    │
├────────────────────────────────────────────────────┤
│  L3  安全层      命令审批 · 路径策略 · 模式门禁      │
├────────────────────────────────────────────────────┤
│  L2  边界层      Tool 注册与执行管线 · 并发 · MCP 桥 │
├────────────────────────────────────────────────────┤
│  L1  引擎层      ReAct · Two-Stage · Reminders · 恢复│
├────────────────────────────────────────────────────┤
│  L0  基底层      LLM Provider · Streaming · 韧性     │
└────────────────────────────────────────────────────┘
```

**L0-L7 是 Harness，L8 是 Loop** —— 两个不同尺度的概念：

| 维度 | Harness（L0-L7） | Loop（L8） |
| --- | --- | --- |
| 作用范围 | **单次调用**内部的工作台：工具、护栏、压缩、可观测性 | **多次调用之间**的外层调度：决定何时再跑一次 |
| 触发者 | 用户打开 REPL 打一行字，或 `--bench` / `-p` 跑一次 | 系统级调度器（launchd / cron）周期性触发 |
| 状态留存 | 会话内的事件日志 | 跨调用持久化到 `~/.seekcli/tasks/*.md` |
| 决策主体 | 模型在 ReAct 循环里自驱思考 + 行动 | 模型仍是决策主体；Rust 只负责「何时调用一次」 |

L8 不给 L0-L7 加新工具或新控制流分支——它只是换一套 prompt，在一个新的触发时机下，
复用同一个 Harness 跑一次完整的 ReAct 循环。

### 逐层文档

| 层 | 文档 | 当前完成度 | 主要欠账 |
| --- | --- | --- | --- |
| L0 | [基底层](L0-llm-substrate.md) | 70% | provider 配置化、重试退避、超时 |
| L1 | [引擎层](L1-engine.md) | 80% | 扩展点、turn/step、可续跑 |
| L2 | [边界层](L2-tools.md) | 45% | MCP、glob/grep、执行管线中间件化 |
| L3 | [安全层](L3-security.md) | 50% | shell 路径策略、Plan Mode 强制、审计 |
| L4 | [记忆层](L4-memory.md) | 45% | **事件日志重构（架构级）** |
| L5 | [组合层](L5-composition.md) | 40% | MCP、子代理可续 |
| L6 | [界面层](L6-interface.md) | 35% | headless 通用化、结构化输出 |
| L7 | [可观测层](L7-observability.md) | 55% | eval 扩容、录制回放、覆盖率门禁 |
| L8 | [循环层](L8-loop.md) | 60% | 任务声明式化 |

横切文档：[设计原则约束](design-principles.md) · [安全边界声明](security-model.md)

---

## 2. 三个必须澄清的心智区分

### 2.1 模板（持久）vs 实例（短暂）

OOP「类 vs 对象」在 Agent 上的复刻：

| 资产 | 模板（注册表，持久） | 实例（运行时，短暂） |
| --- | --- | --- |
| Tool | 代码里定义的函数 + JSON schema | 一次函数调用，即生即灭 |
| Skill | `~/.seekcli/skills/<name>/SKILL.md` | 激活到 `/clear` 为止 |
| SubAgent 类型 | 代码里注册的 `SubAgentTemplate` | — |
| SubAgent 实例 | — | 一次 `invoke_agent` 调用 |
| Session | — | 一次 REPL 会话 |

**LLM 临时决定的只是「用哪个模板 + 怎么调用 + 何时停止」，从不凭空发明新模板。**

### 2.2 ReAct 范式里没有独立的 Planner Agent

```
主 Agent 的一次循环迭代：
  think     ← 这就是「规划」  ┐
  tool_call ← 这就是「执行」  ├ 同一次 LLM 调用完成
  observe   ← 拿到工具结果    │
  think     ← 下一轮规划      ┘
```

不要被 LangGraph 系术语误导：Hermes / Claude Code / SeekCLI 都是纯 ReAct。
「规划」是模型在 reasoning 阶段自驱完成的步骤，**不是另起一个 agent**。

> Two-Stage ReAct（L1）是例外中的例外：它是**同一个 agent** 的一次不带工具的额外调用，
> 用于在动手前强制思考，仍然不是独立 Planner。

### 2.3 SubAgent 是「运行时上下文压缩」的具体形态

| 机制 | 压缩时机 | 收益 |
| --- | --- | --- |
| Skill | 会话启动时 | 缩窄初始上下文（少注入无关工具） |
| SubAgent | 任务执行中 | 隔离子任务，只带摘要回主轴 |
| Context Compressor | 主轴超阈值时 | 把中段对话压成摘要 |

三者是**同一族技术**在不同时间点的应用。

---

## 3. 模块布局

```
src/
├── main.rs              薄壳：App struct + new + run(REPL) + Cli + main
├── engine.rs            impl App：ReAct 主循环 + Two-Stage + Reminders/Recovery
│                        + Fork-Join 分发 + SubAgent 委派 + chat/headless
├── commands.rs          impl App：slash 命令分发 + help + skill 激活 + /copy
├── benchmark.rs         impl App：Benchmark 编排
├── completer.rs         rustyline Tab 补全（无 App 耦合）
├── api/                 L0：provider 中立 schema + OpenAI/Anthropic 双 wire
├── agent/               L1/L4：prompt · compressor · reminders · recovery
├── tools/               L2/L3：registry · dispatcher · approval · path_security
│                        · offload · fs · edit · shell · meta
├── subagents/           L5：SubAgent 类型注册表
├── observability/       L7：cost · trace · bench
├── skills.rs            L5：SKILL.md 模板持久化 + proposal 审核
├── history.rs           L4：Session 持久化
├── tasks.rs             L8：--run-task 分发 + digest 提醒
└── config.rs            配置加载
```

> `engine.rs` / `commands.rs` / `benchmark.rs` 都是 `impl App` 的分块——
> 利用 Rust「子模块可见祖先私有项」规则，逻辑零改动地把 main.rs 从 1450 行拆到 227 行。
> App struct 仍定义在 crate root，故各子模块均可访问其私有字段。

---

## 4. 与 Hermes Agent 的对位

参考：<https://github.com/NousResearch/hermes-agent>（Python）。

| 维度 | Hermes 做法 | SeekCLI 策略 |
| --- | --- | --- |
| 主循环 | 纯 ReAct，单进程 | 同范式 |
| 工具注册 | import-time self-register + 派发模块 | Rust 静态 `system_tools()` + `ToolDispatcher` |
| 工具数量 | 70+（含浏览器 / MCP / voice） | 核心 8 个，其余靠 MCP 接入（L5） |
| SubAgent | 独立 delegate 模块 | `subagents/registry.rs` 类型化模板 + 深度限制 |
| Skill 来源 | 人工策展的 bundle | `<name>/SKILL.md` + `create_skill` 走 proposal 审核 |
| 自演化 | 拆到独立仓库离线优化 | **不做在线自演化** |
| 上下文压缩 | `context_compressor.py` | `agent/compressor.rs` 阶梯降级 |
| 安全护栏 | `approval.py` + `path_security.py` | 同名对位模块 |
| 记忆 | Honcho dialectic + FTS5 跨会话 | **不做跨会话语义记忆** |

**真实启发**收敛为五条：工具 schema 必须注入 LLM；危险命令必须有审批；
SubAgent 必须类型化；Skill 必须策展；长会话必须有压缩。

---

## 5. 与 deepseek-harness 的关系

`deepseek-harness`（dsh）是本项目当前的**组件验收清单**，不是模仿对象。

- **借鉴**：capability seam（接口 / 实现 / 消费者三角色）、model-visible means logged（事件日志）、
  工具面广度、生成式文档 + CI 新鲜度门禁。
- **不借鉴**：插件框架（Cordis）、profile/bundle 组合、Web UI、多语言 SDK、i18n。
  这些的复杂度收益比在单人 Rust CLI 上不成立。

### 5.1 不借鉴 Cordis 的精确理由

早期的理由写作「太复杂」，这不是一条可判定的判据。读过 vendored 源码
（`vendor/cordis/src/`，2693 行核心）后可以说得更准：

> **Cordis 的核心不是「插件」或「服务注册表」，是 `Fiber`——一个带可逆副作用的
> 生命周期状态机（754 行，占核心 28%）。它本质上是在 GC 语言里手工重建
> RAII + 作用域生命周期，而 Rust 编译器免费提供这个。**

三条核心机制的对位：

| Cordis 机制 | SeekCLI 的答案 |
| --- | --- |
| Effect 可逆（注册必带 disposer，卸载等 quiescence） | **已天然拥有**：`Drop` + 所有权 + 作用域。Cordis 需要运行时检查拦截的「清理期注册逃出卸载快照」，Rust 里编不过 |
| 响应式依赖（`inject` 是订阅，依赖消失则退回 PENDING） | 能做，但需真有第二个实现才值得（判据见 [design-principles §5.1](design-principles.md#51-三问背后的判据)） |
| 运行期加载新代码 | **Rust 拿不到**（ABI 不稳定、卸载几乎必然 UB）；等价需求由 MCP 的**进程边界**满足——kill 即完整 quiescence |

完整论证见 [2026-09-12 三评 §4](../evaluation/2026-09-12-self-evolution-baseline.md#4-dsh-的架构核心读源码而非读文档的结论)。

判断标准见 [design-principles.md](design-principles.md)。
