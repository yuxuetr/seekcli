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

## 4. SeekCLI 当前分层评估（阶段八完成后）

| 层               | 状态  | 说明                                                                      |
| ---------------- | ----- | ------------------------------------------------------------------------- |
| L0 LLM 基底      | ✅    | DeepSeek + streaming + ToolCall 协议解析；streaming tool-call 分片重组已修 |
| L1 引擎          | ✅    | ReAct + MAX_ITER=25 + MAX_SUBAGENT_DEPTH=3；Ctrl-C 中断推迟到阶段十一      |
| L2 工具          | ✅    | `tools/registry::system_tools()` 全量 schema 注入；6 个内置工具齐备         |
| L3 安全          | ✅    | `approval.rs` 危险命令审批 + `path_security.rs` write 路径白名单（见 §8）  |
| L4 记忆          | 🔴    | 仅有 session JSON，无压缩、无 prompt cache 验证 → 阶段十                    |
| L5 组合          | ⚠️    | SubAgent depth + 工具裁剪已落地，类型化模板与 Skill proposal 待阶段九       |
| L6 界面          | ✅    | REPL 干净，渲染框已删（阶段七）；进度条 + Ctrl-C 留阶段十一                 |
| **外围资产**     | ✅    | MinerU/StepFun VLM/Tavily/GLM Search/Jina 已全部剥离（阶段七）              |

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

---

## 8. 安全边界声明 (Security Model Boundary)

SeekCLI 是**单用户本地 CLI**，不是生产服务。L3 安全层的设计是
"在工具结构允许的范围内提供常识性护栏"，而不是"完整 OS 隔离"。

### 8.1 L3 实际拦截清单

**`tools/approval.rs::is_dangerous`** 通过 token 边界匹配识别危险命令模式：
- `rm -rf` 触及 `/` | `~` | `$HOME` | 任何绝对路径
- `sudo` 提权
- `curl|sh` / `wget|bash` 远程脚本管道
- `dd of=/dev/...` 块设备写
- fork bomb `:(){...}`
- `chmod 777` / `chmod -R 777`
- `git push --force` / `git push -f`
- `mkfs*` 文件系统格式化

命中后在 stderr 弹 `y/N` 提示，用户拒绝则返回 `[USER DENIED]` 前缀给模型，
系统提示明确要求"见到 DENIED 不要重试同一调用"。

**`tools/path_security.rs::ensure_within_cwd`** 通过 lexical normalize 防止
`write_file` 路径越权。绝对路径或 `../..` 逃逸到 cwd 之外被拒，返回
`[PATH DENIED]` 前缀。

### 8.2 L3 明确**不**拦截的内容

**shell 重定向到 cwd 之外**：`run_shell("date > /tmp/foo")` 会成功执行。
原因：shell 写文件的方式无穷多（`>` / `>>` / `tee` / `cp` / `mv` /
heredoc / `cat > /foo` ...），可靠拦截需要解析 shell AST，与"轻量 CLI"
定位相悖。这是**已知的设计选择**，不是 bug。

**`read_file` / `list_dir` 跨目录**：模型可以读 `~/.zshrc` 等任意可读文件。
原因：`run_shell` 已有同等 exfiltration 能力（`cat ~/.ssh/id_rsa`），单独
限制 read 路径只损体验不增实际安全。

**网络访问 / 子进程派生 / 数据外发**：进程级别无任何限制。
原因：需要 OS 级 sandbox（chroot / Docker / firejail / seccomp），与
"用户本地 CLI"定位冲突。如需该级别隔离，请在容器内运行 SeekCLI。

### 8.3 横向对比

| 项目 | 危险命令审批 | fs 路径检查 | shell 完全沙箱 |
|---|---|---|---|
| **SeekCLI** | ✅ token 匹配 | ✅ write_file 限 cwd | ❌ 不做 |
| **Claude Code** | ✅ 用户允许列表 | ✅ write 限 workspace | ❌ 不做 |
| **Hermes Agent** | ✅ `approval.py` (58KB) + `tool_guardrails.py` | ✅ `path_security.py` | ❌ 不做 |
| **OpenAI Codex CLI** | ✅ 类似 | ✅ 类似 | ⚠️ macOS sandbox-exec 可选 |

业界主流方案与 SeekCLI 一致：**工具层做语义护栏，不在 CLI 层做完整 OS 沙箱**。

### 8.4 用户责任声明

启动 SeekCLI 等同于**给一个智能模型授予终端访问权限**。用户应理解：

1. **审计 stdout**：`run_shell` 的命令在执行前会打印
   `[Agent Executing] <command>`，用户应当注意阅读。
2. **不在敏感目录里运行**：避免在 `~/.ssh/`、`~/Documents/财务/` 等目录
   直接 `cd` 后启动 SeekCLI。
3. **重要数据先备份**：与所有自动化工具一样。
4. **如需更强隔离**：在 Docker 容器内运行 SeekCLI，并 mount 仅需要的
   目录为只读 / 读写。

### 8.5 未来增强方向（不在当前路线图）

- OS sandbox 集成（macOS `sandbox-exec` / Linux seccomp / firejail）
- 操作审计日志（落到 `~/.seekcli/audit.log`，便于事后追溯）
- `run_shell` 的可选只读模式（仅允许在白名单命令集合内）

这些都是独立的工程项目，不计入七层架构核心，也不在当前 TODOs 推进计划中。
