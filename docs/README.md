# SeekCLI 文档

```
docs/
├── architecture/   怎么设计的、每层要长成什么样   ← 改代码前读这里
├── evaluation/     还缺什么、值不值得补           ← 排优先级时读这里
└── archive/        历史留痕，仅供回溯
```

## 三类文档的分工

```
docs/evaluation/    发现缺口，给优先级
        │
        ▼
docs/architecture/  为每个缺口设计补全方案
        │
        ▼
TODOs.md            把方案拆成可执行、可验收的阶段
```

**任何一处缺口都应该能沿这条链找到出处。**
评估里的编号（如 `L4-1`）在架构文档里有对应的目标设计，在 `TODOs.md` 里有对应的阶段。

这条链由 `scripts/check-gap-coverage.py` 在 CI 与 pre-commit 中**强制**，三向校验：

- 评估里定义的每个缺口，必须落进三个桶之一：**已排期** / **已知仍开放** / **明确不排期（P3）**；
- `TODOs.md` / 架构文档引用的每个编号，必须在评估里有定义，否则无法追溯到证据；
- 层文档缺口表里**仍标为开放**的编号，路线图里也必须仍是未完成状态——反之亦然。

三个方向都有过真实漏网，所以是机械校验而非人工约定：

| 漏网 | 表现 |
| --- | --- |
| L2-7 | 有完整设计，却没有任何阶段承接，而另一个阶段已假设它存在 |
| L6-5 | 有阶段在做，评估里却没有定义，追溯会断链 |
| L7-1 / L7-4 | 工作早已完成，层文档的缺口表却还开着——因为没有东西比对两边 |

## 索引

### 架构 —— [`architecture/`](architecture/README.md)

| 文档 | 内容 |
| --- | --- |
| [总纲](architecture/README.md) | 项目定位、八层心智模型、三个心智区分、模块布局 |
| [L0 基底层](architecture/L0-llm-substrate.md) | Provider 抽象、Streaming、重试与超时 |
| [L1 引擎层](architecture/L1-engine.md) | ReAct 主循环与运行时纠偏机制 |
| [L2 边界层](architecture/L2-tools.md) | 工具注册与执行管线 |
| [L3 安全层](architecture/L3-security.md) | 审批、路径策略、模式门禁 |
| [L4 记忆层](architecture/L4-memory.md) | 会话事件日志、压缩、状态外部化 |
| [L5 组合层](architecture/L5-composition.md) | SubAgent、Skill、MCP |
| [L6 界面层](architecture/L6-interface.md) | REPL、CLI、headless 入口 |
| [L7 可观测层](architecture/L7-observability.md) | Cost、Tracing、Benchmark |
| [L8 循环层](architecture/L8-loop.md) | 系统级调度与定时任务 |
| [设计原则约束](architecture/design-principles.md) | **明确排除了什么** |
| [安全边界声明](architecture/security-model.md) | 拦截什么、不拦截什么、用户责任 |

每篇层文档统一按 **职责边界 → 当前实现 → 缺口 → 目标设计 → 明确不做 → 验收标准 → 对应路线** 组织。

### 评估 —— [`evaluation/`](evaluation/README.md)

| 文档 | 内容 |
| --- | --- |
| [评分口径](evaluation/scoring-rubric.md) | 三个正交口径与缺口分级。**先读这个** |
| [2026-08 dsh 对照评估](evaluation/2026-08-harness-gap-analysis.md) | 当前基线：整体约 40% |

### 归档 —— [`archive/`](archive/)

| 文档 | 内容 |
| --- | --- |
| [阶段一 ~ 十九路线图](archive/TODOs-phase-01-19.md) | 2026-08-22 归档，含每阶段 commit 号与验收记录 |
