# 设计原则约束

> 写代码时与本文件保持一致。**新增约束需要同时说明它排除了什么**，
> 只说「要做好」的不是约束。

## 1. 能力归属

- **凡是「客户端预注入」的能力都应改造为 Tool。** 不再新增 `@xxx` 这类 client-side 解析路径。
  能力应由 Agent 通过 Tool Calling 自取（阶段七剥离 MinerU / VLM / Tavily / GLM Search 的依据）。
- **L8 不新增 Rust 工具**：`--run-task` 只是换一套 prompt 驱动同一个 `run_headless` / ReAct 循环。

## 2. 明确排除的复杂度

| 排除项 | 理由 |
| --- | --- |
| 跨会话语义记忆 | 与 CLI 即时性目标背离；session 事件日志 + 工作区 PLAN.md 已覆盖 |
| 在线自演化 Skill | 模型只能起草 proposal，落地必须人工审核 |
| 运行时多模型路由 / 负载均衡 | provider 由配置**静态**选择。支持多 wire 协议 ≠ 做调度器 |
| plan-execute / multi-agent 框架 | 纯 ReAct + 类型化 SubAgent 已足够 |
| 插件框架 / profile / bundle 组合 | 单人 Rust CLI 上复杂度收益比不成立；扩展需求由 MCP 承担 |
| DAG / workflow 编排 | 是另一个产品 |
| TUI / Web UI | 与「本地 CLI Agent」定位冲突 |
| 完整 shell AST 解析 | 见 [security-model.md](security-model.md) §2 |

> **Plan Mode 不是 plan-execute 框架。** 它是「状态外部化」+「模式门禁」：
> 模型自驱把规划写进工作区 PLAN.md / TODO.md，控制流仍是纯 ReAct，
> 不引入独立 Planner Agent 或 DAG 调度器。

> **Two-Stage ReAct 不是独立 Planner。** 它是同一个 agent 的一次不带工具的额外调用。

## 3. 结构约束

- **可观测性走装饰器 / 旁路，不污染主控流。**
  CostTracker / Resilient / Recording 都包装 `LlmProvider`；
  Tracing 在 engine / registry 边界埋点。
  **`run_agent_loop` 里不得出现计费、重试、录制代码。**
- **任何超过 50 行的渲染美化逻辑必须独立成模块**，不得污染 agent loop。
- **`impl App` 可以分块到子模块**（`engine.rs` / `commands.rs` / `benchmark.rs`），
  利用 Rust「子模块可见祖先私有项」规则，避免 main.rs 膨胀。
- **模型可见 = 已记录**（L4 事件日志落地后生效）：
  任何进入模型请求的内容必须能从 `events.jsonl` 重建。

## 4. 错误处理

遵循全局 Rust 规范，本项目额外强调：

- **不用 `unwrap()` / `expect()`**（测试、示例、构建脚本除外）。
  遇到时先解决根因：为什么这个 `Option` 可能是 `None`？能否从类型设计上消除？
- **降级优于中断**：压缩失败 → 日志告警 + 继续原 messages；
  offload 写盘失败 → 内联预览；MCP server 启动失败 → 跳过该 server。
  **但绝不静默**——每次降级必须有一行可见输出。
- **不静默截断**：超上限的结果必须显式告知「还有 N 条未显示」。

## 5. 与 deepseek-harness 的边界

dsh 是**组件验收清单**，不是模仿对象。判断某项要不要对齐，问三个问题：

1. 它解决的问题在单人本地 Rust CLI 上真实存在吗？
   （Web UI / i18n / 多语言 SDK → 不存在）
2. 它的复杂度能被 5000 行量级的项目承担吗？
   （Cordis 插件框架 → 不能）
3. 有没有更轻的等价方案？
   （第三方扩展：插件框架 → **MCP**；循环扩展点：事件总线 → **静态 `Vec<Box<dyn LoopHook>>`**）

三问全过才对齐。**对齐 dsh 本身不是目标。**

## 6. 变更本文件

新增或删除约束时，同步更新 [`docs/architecture/README.md`](README.md) 的相关表述，
并在 `TODOs.md` 里记录触发这次变更的阶段号。
