# SeekCLI: Harness Agent 架构纲领

> 本文是 SeekCLI 演进的"宪法级"文档，定义心智模型与分层职责。
> 路线落地见 `TODOs.md`；对外定位见 `README.md`。

---

## 0. 项目重新定位 (Re-positioning)

**SeekCLI = DeepSeek V4 + Tools + Harness Agent 核心**。

它是一个**单纯的本地 CLI Agent**，不做多模态网关、不做内容平台胶水。
所有"外围感官"（MinerU/StepFun VLM/Tavily/Jina/GLM Search）将从核心中剥离：
模型需要的能力应通过 **Tool Calling** 由 Agent 自取，而不是由 CLI 客户端预先注入。

---

## 1. 智能体心智模型：七层架构

```
┌────────────────────────────────────────────────────┐
│  L6  界面层      REPL · CLI 子命令                  │
├────────────────────────────────────────────────────┤
│  L5  组合层      SubAgent 模板 · Skill 策展 · MCP   │
├────────────────────────────────────────────────────┤
│  L4  记忆层      上下文压缩 · prompt cache · 会话    │
├────────────────────────────────────────────────────┤
│  L3  安全层      命令审批 · 路径白名单 · 工具守卫    │
├────────────────────────────────────────────────────┤
│  L2  边界层      Tool Dispatcher · 工具 schema 注册 │
├────────────────────────────────────────────────────┤
│  L1  引擎层      ReAct Loop · 迭代上限 · 中断处理    │
├────────────────────────────────────────────────────┤
│  L0  基底层      LLM Provider · Streaming · 协议解析 │
└────────────────────────────────────────────────────┘
```

**核心判断**：
- **L0/L1** 是必要骨架，写得快（合计 ~500 行）；
- **L3/L4/L5** 是工程深水区，决定"玩具 demo 还是日常可用工具"的分水岭；
- **外围资产**（多模态网关）不在七层之内 —— 它们应被工具化或剥离。

---

## 2. 三个必须澄清的心智区分

### 2.1 模板（持久）vs 实例（短暂）

这是 OOP "类 vs 对象" 在 Agent 上的复刻：

| 资产           | 模板（注册表，持久）              | 实例（运行时，短暂）           |
| -------------- | --------------------------------- | ------------------------------ |
| Tool           | 代码里定义的函数 + JSON schema    | 函数调用，即生即灭             |
| Skill          | `~/.seekcli/skills/*.json`        | 激活到 `/clear` 为止           |
| SubAgent 类型  | 代码里注册的 `SubAgentTemplate`   | —                              |
| SubAgent 实例  | —                                 | 一次 `invoke_agent` 调用       |
| Session        | —                                 | 一次 REPL 会话                 |

**LLM 临时决定的只是"用哪个模板 + 怎么调用 + 何时停止"，从不凭空发明新模板。**

### 2.2 ReAct 范式里**没有独立的 Planner Agent**

```
主 Agent 的一次循环迭代：
  think    ← 这就是"规划"  ┐
  tool_call ← 这就是"执行"  ├ 同一次 LLM 调用完成
  observe  ← 拿到工具结果   │
  think    ← 下一轮规划     ┘
```

不要被 LangGraph 系的术语误导：Hermes / Claude Code / SeekCLI 都是纯 ReAct，
"规划"是模型在 `<think>` / `reasoning_content` 阶段自驱完成的步骤，**不是另起一个 agent**。

### 2.3 SubAgent 是"运行时上下文压缩"的具体形态

| 机制                | 压缩时机        | 收益                            |
| ------------------- | --------------- | ------------------------------- |
| Skill               | 会话启动时      | 缩窄初始上下文（少注入无关工具）|
| SubAgent            | 任务执行中      | 隔离子任务，只带摘要回主轴       |
| Context Compressor  | 主轴超阈值时    | 把中段对话压成摘要              |

三者是**同一族技术**在不同时间点的应用。

---

## 3. 与 Hermes Agent 的真实对位

参考仓库：<https://github.com/NousResearch/hermes-agent>（Python，152k★）。

| 维度        | Hermes 真实做法                                                     | SeekCLI 对应策略                                       |
| ----------- | ------------------------------------------------------------------- | ------------------------------------------------------ |
| 主循环      | 纯 ReAct，`AIAgent.run_conversation` 单进程                          | 同范式，沿用                                           |
| 工具注册    | `tools/registry.py` import-time self-register；`model_tools.py` 派发 | Rust 静态 `system_tools()` + `ToolDispatcher` 派发     |
| 工具数量    | 70+ 个（含浏览器/MCP/voice 等）                                      | 仅核心 5 ~ 8 个（fs/shell/meta），其余靠 MCP 后续接入  |
| SubAgent    | `tools/delegate_tool.py` 单独大模块                                  | `subagents/registry.rs` 类型化模板，深度限制           |
| Skill 来源  | **人工策展的 bundle**（`skills/{github,devops,...}`）                | 默认内置策展 + `create_skill` 走 **proposal 审核**     |
| 自演化      | 拆到独立仓库 `hermes-agent-self-evolution`（DSPy/GEPA 离线优化）     | **不做在线自演化**，保持核心纯粹                       |
| 上下文压缩  | `agent/context_compressor.py` 74KB                                   | `agent/compressor.rs` 阈值触发摘要                     |
| 安全护栏    | `tools/approval.py` 58KB + `path_security.py` 等                    | `tools/approval.rs` + 路径白名单                       |
| 记忆        | Honcho dialectic + FTS5 跨会话                                       | **不做**，session JSON 已足够                          |

**Hermes 给 SeekCLI 的真实启发**应缩减为：
1. 工具 schema 必须注入 LLM（静态注册表模式）；
2. 危险命令必须有审批护栏；
3. SubAgent 必须类型化；
4. Skill 必须策展（在线自演化拆出主流程）；
5. 长会话必须有压缩机制。

---

## 4. SeekCLI 当前分层评估（修订版）

| 层               | 状态  | 说明                                                                      |
| ---------------- | ----- | ------------------------------------------------------------------------- |
| L0 LLM 基底      | ✅    | DeepSeek + streaming + ToolCall 协议解析完整                               |
| L1 引擎          | ⚠️    | ReAct 已实现，但缺 max_iter / 中断处理                                     |
| L2 工具          | 🔴    | 工具实现存在，但 **schema 未注入 LLM**，dispatcher 形同虚设                |
| L3 安全          | 🔴    | `run_shell` 直接执行无审批；fs 工具无路径白名单                            |
| L4 记忆          | 🔴    | 仅有 session JSON，无压缩、无 prompt cache                                 |
| L5 组合          | ⚠️    | SubAgent 实现存在但无类型化、无深度限制；Skill 定位需修正                  |
| L6 界面          | ✅    | REPL 完整；花哨渲染应抽出或删除                                            |
| **外围资产**     | 🟡    | MinerU/StepFun VLM/Tavily/GLM Search/Jina 等占代码量 ~30%，应剥离          |

---

## 5. 模块目标布局（演进后）

```
src/
├── main.rs              REPL + CLI 入口（< 200 行）
├── api.rs               DeepSeek client + StreamItem（< 250 行）
├── agent/
│   ├── mod.rs           run_agent_loop（含 max_iter / 深度）
│   ├── prompt.rs        Agent 系统提示构建
│   └── compressor.rs    上下文压缩（L4）
├── tools/
│   ├── mod.rs           ToolDispatcher
│   ├── registry.rs      工具 schema 注册（L2 核心）
│   ├── approval.rs      危险命令审批（L3）
│   ├── path_security.rs 路径白名单（L3）
│   ├── fs.rs            read_file / write_file / list_dir
│   ├── shell.rs         run_shell（带 approval 钩子）
│   └── meta.rs          invoke_agent / create_skill
├── subagents/
│   └── registry.rs      SubAgent 类型注册表（L5）
├── skills.rs            模板持久化 + proposal 审核
├── history.rs           Session 持久化
└── config.rs
```

---

## 6. 演进路线指引

具体任务拆解见 `TODOs.md`，按以下阶段推进：

1. **阶段六：Harness 核心修补**（L1/L2 致命缺口）
2. **阶段七：外围资产剥离**（砍 MinerU/VLM/Tavily/GLM Search 等）
3. **阶段八：L3 安全层**（审批 + 路径白名单）
4. **阶段九：L5 组合层升级**（SubAgent 类型化 + Skill proposal）
5. **阶段十：L4 记忆层**（压缩 + prompt cache）
6. **阶段十一：L6 界面瘦身**（渲染抽离，纯文本优先）

---

## 7. 设计原则约束

写代码时遵循以下原则，与本文档保持一致：

- **凡是"客户端预注入"的能力都应改造为 Tool**：不要再加 `@xxx` 这种 client-side 解析路径。
- **不引入跨会话语义记忆**：与 CLI 即时性目标背离。
- **不做在线自演化 skill**：模型只能起草 proposal，落地必须人工审核。
- **不做多 LLM provider 调度**：DeepSeek V4 单家深度适配。
- **不引入 plan-execute / multi-agent 框架**：纯 ReAct + 类型化 SubAgent 已足够。
- **任何长度超过 50 行的"渲染美化"逻辑必须独立成模块**，不得污染 agent loop。
