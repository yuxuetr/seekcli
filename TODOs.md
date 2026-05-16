# SeekCLI 进化路线图 (TODOs)

> 项目重新定位：**单纯的本地 CLI Agent**，核心 = DeepSeek V4 + Tools + Harness Agent 引擎。
> 心智模型与分层职责见 `AGENT_ARCHITECTURE.md`。

---

## ✅ 阶段一：API 协议重构 (兼容多模态)
- [x] 重构 `Message` 结构，支持多模态 ContentPart。
- [x] 多 Provider 适配器抽象 (DeepSeek, Zhipu, MinerU 等)。

## ✅ 阶段二：视觉理解管线 (Vision Pipeline)
- [x] 剪贴板位图捕获及后台解析。
- [x] 视觉预处理并注入到推理上下文中。

## ✅ 阶段三：文件解析与 `@` 语法 (File Sense)
- [x] `@` 语法解析器实现。
- [x] 集成 MinerU 进行复杂文档 (PDF/Docx) 解析。

## ✅ 阶段四：联网搜索插件 (Global Tools)
- [x] 接入 GLM Web Search 与 Tavily AI Search。

## ✅ 阶段五：Harness Agent 核心引擎构建（**有已知缺口，见阶段六**）
- [x] 5.1 构建本地工具分发器 (Tool Dispatcher)
- [x] 5.2 重构 Agent 核心闭环 (The ReAct Loop)
- [x] 5.3 实现 Sub-Agent (子智能体委派) 机制
- [x] 5.4 动态技能生成体系（**已知问题：在线自演化易污染，将在阶段九重定位为 proposal 审核流程**）

> ⚠️ **已知缺口**：
> - 工具实现存在，但 schema 未注入 LLM，dispatcher 在生产路径下未被触发；
> - ReAct loop 无 max_iter，存在烧 token 风险；
> - SubAgent 无类型化、无深度限制，工具集未裁剪导致递归风险；
> - 缺 Agent 系统提示，模型不知道自己处于 Harness 环境。
>
> 上述缺口在 **阶段六** 集中修补。

---

## ✅ 阶段六：Harness 核心修补 (Step 1)
*目标：用最小补丁让现有 ReAct loop 真正运转。不删任何已有代码，仅新增。*

- [x] **6.1 工具 schema 注册表** — `src/tools/registry.rs`
    - [x] `system_tools() -> Vec<Tool>` 注册 read_file / write_file / list_dir / run_shell / invoke_agent / create_skill 全部带 JSON schema。
    - [x] `merge_with_skill()` 合并 skill tools，系统工具同名优先。
    - [x] `run_agent_loop` 入口（depth=0 时）调用 `merge_with_skill` 后下发。
- [x] **6.2 Agent 系统提示构建器** — `src/agent/prompt.rs`
    - [x] `agent_system_prompt()` 覆盖身份、ReAct 循环说明、工具用法、停止条件。
    - [x] `subagent_preamble(depth)` 子 agent 启动时拼接说明。
    - [x] `Self::ensure_agent_system_prompt()` 在 `chat()` 入口和子 agent 启动时保证 system message 在头部。
- [x] **6.3 迭代上限** — `src/agent/mod.rs::MAX_ITER = 25`
    - [x] `run_agent_loop` 用 `for _iter in 0..MAX_ITER` 替代裸 `loop`。
    - [x] 达到上限打印 `[Agent: reached max iterations]` 并安全退出。
    - [ ] REPL 用 `tokio::select!` + `ctrl_c()` 支持中断（**推迟到阶段十一**，需要重构 streaming）。
- [x] **6.4 SubAgent 深度限制与工具裁剪**
    - [x] `MAX_SUBAGENT_DEPTH = 3`，`run_agent_loop` 顶部 bail。
    - [x] `tools::registry::filter_for_subagent()` 过滤 `invoke_agent / create_skill`，防止递归与越权。
    - [x] `run_agent_loop` 增加 `depth: usize` 参数，子 agent 调用 `next_depth = depth + 1`。

**验收方式**：`cargo test` 全过 + `cargo clippy --no-deps` 无告警 ✅。
**实战验证**：运行 `cargo run` → 输入 "请读 Cargo.toml 并告诉我用了哪些依赖" → 观察 DeepSeek 主动调用 `read_file`。

---

## 🪓 阶段七：外围资产剥离 (Step 2)
*目标：去掉所有非 Harness 资产，回归"纯 CLI Agent"。独立 PR，方便回滚。*

- [ ] **7.1 删除多模态 sensor**
    - [ ] `api.rs` 移除 `Provider::{Zhipu, DashScope, MinerU, StepFun}` 分支。
    - [ ] `api.rs` 移除 `MineruResponse / mineru_extract / mineru_get_result / fetch_url_content / fetch_web_markdown / tavily_search / glm_web_search`。
    - [ ] `api.rs` 移除 `Message::new_user_image / new_user_file` 多模态构造器。
    - [ ] `main.rs` 移除 `vlm_sensor / doc_sensor / glm_sensor` 字段与初始化。
    - [ ] `main.rs` 移除 `analyze_complex_file / paste_image`。
- [ ] **7.2 删除外围 slash command**
    - [ ] 移除 `/image` `/file` `/web` `/search` `/tavily`。
    - [ ] `chat()` 中移除 `@image / @url / @search / @tavily / @pdf` 客户端解析。
    - [ ] 模型需要这些能力请走 `run_shell("curl ...")` 或后续接入的 MCP 工具。
- [ ] **7.3 删除 auto-route**
    - [ ] 移除 `route_skill` 方法及其在 REPL 主循环的调用。
    - [ ] 让模型在 ReAct 里通过 `load_skill` 工具（待实现）自取。
- [ ] **7.4 渲染层瘦身**
    - [ ] 抉择：保留 termimad/syntect 抽到 `src/render.rs`，**或** 删除走纯文本输出。
    - [ ] 推荐方案：删除，与 Harness Agent 极简风格一致。
- [ ] **7.5 依赖清理**
    - [ ] `Cargo.toml` 移除 `image / base64 / mime_guess / bytes`。
    - [ ] 若删渲染：再移除 `termimad / syntect`。

**验收**：`api.rs < 250 行`；`main.rs < 500 行`；`cargo build --release` 通过；外围环境变量不再出现在文档与代码。

---

## 🛡️ 阶段八：L3 安全层
*目标：让 `run_shell` 从"玩具"变"日常可用"。*

- [ ] **8.1 危险命令审批**
    - [ ] 新增 `src/tools/approval.rs`。
    - [ ] 拦截规则：`rm -rf /|~|$HOME`、`sudo`、`curl | sh`、`dd of=/dev/`、fork bomb、`chmod 777`、`git push --force`。
    - [ ] 命中时终端打印命令 + 原因，要求 `y/N` 显式确认。
    - [ ] 用户拒绝时返回 `"User denied execution of: ..."` 给 LLM，让其自我调整。
- [ ] **8.2 fs 路径白名单**
    - [ ] 新增 `src/tools/path_security.rs::check_path()`。
    - [ ] `write_file` 强制约束在 `current_dir` 子树（除非 skill 显式放宽）。
    - [ ] `read_file` 默认放宽，但记录访问日志。
- [ ] **8.3 工具结果分类**
    - [ ] `ToolDispatcher::execute` 返回 `ToolResult { kind: Success|UserDenied|Error, content: String }`。
    - [ ] 不同 kind 在 prompt 里用不同标签，帮助模型理解失败原因（参考 Hermes `tool_result_classification.py`）。

**验收**：让模型尝试 `rm -rf ~` 时被拦下；让模型尝试在 `/tmp` 之外写文件时被路径白名单拒绝。

---

## 🧩 阶段九：L5 组合层升级
*目标：SubAgent 类型化，Skill 重定位为 proposal 审核流程。*

- [ ] **9.1 SubAgent 模板注册表**
    - [ ] 新增 `src/subagents/registry.rs`，定义 `SubAgentTemplate { name, system_prompt, allowed_tools, max_iter }`。
    - [ ] 内置至少两种：`explore`（只读探索）、`general`（通用子任务）。
    - [ ] `invoke_agent` 工具 schema 加 `subagent_type` 枚举字段。
    - [ ] 派发时按 template 设置 system prompt 与工具子集。
- [ ] **9.2 Skill proposal 审核机制**
    - [ ] `create_skill` 工具写入 `~/.seekcli/skills/proposals/` 而非直接 `skills/`。
    - [ ] 新增 `/skill review` REPL 命令：列出 proposals → 用户 `accept/reject/edit`。
    - [ ] 通过审核的 skill 才移动到 `~/.seekcli/skills/` 生效。
    - [ ] 文档明确说明：这是协作流程，不是"自演化"。
- [ ] **9.3 Skill 加载工具化**
    - [ ] 新增 `load_skill` 系统工具，让模型在 ReAct 里按需切换技能（替代被删的 auto-route）。
    - [ ] schema：`{"name": "translator"}`。
    - [ ] 切换时把 skill 的 system_prompt 作为新 system message 推入上下文。

**验收**：模型可通过 `invoke_agent("explore", "find all use of tokio::spawn")` 拿到摘要；模型起草的 skill 必须经过用户审核才能生效。

---

## 🧠 阶段十：L4 记忆层
*目标：让长会话不爆 token，命中 prompt cache。*

- [ ] **10.1 上下文压缩**
    - [ ] 新增 `src/agent/compressor.rs::maybe_compress()`。
    - [ ] 阈值：`messages` 总字符数 > 600KB（约 150K token）时触发。
    - [ ] 策略：保留 system + 最近 6 条，中间段交给 DeepSeek 自己摘要。
    - [ ] 摘要结果以特殊 user message `[Compressed earlier turns] ...` 插入。
- [ ] **10.2 prompt cache 验证**
    - [ ] 固定 system message 完全一致（不随时间戳变化）。
    - [ ] streaming 结束时打印 `prompt_cache_hit_tokens`，验证命中率。
- [ ] **10.3 Session 改造**
    - [ ] 压缩前的原始 messages 落 `sessions/<id>.raw.json`，方便回溯。
    - [ ] 压缩后的 messages 落 `sessions/<id>.json`，作为 `/load` 复活点。

**验收**：模拟一次 30 轮对话，触发压缩后 token 总量回落 60%+，且后续推理仍能引用早期内容的关键点。

---

## 🎨 阶段十一：L6 界面瘦身
*目标：让界面层只做"REPL + 必要状态指示"，不抢 Agent 戏。*

- [ ] **11.1 渲染解耦**
    - [ ] 若阶段七保留了渲染，将其全部抽到 `src/render.rs`。
    - [ ] `run_agent_loop` 内部不持有 `MadSkin` / `SyntaxSet` 等渲染句柄。
- [ ] **11.2 状态指示**
    - [ ] 用 `indicatif` 显示 "Calling tool: ..." / "Sub-agent depth=N" 状态。
    - [ ] 工具执行时间 > 3s 显示进度。
- [ ] **11.3 命令补全**
    - [ ] `rustyline` 注册 slash command 自动补全。
    - [ ] 注册 skill / subagent 名字补全。

---

## 📌 三项待用户拍板的决策

| # | 决策                       | 默认方案（推荐）                | 备选                       |
| - | -------------------------- | ------------------------------- | -------------------------- |
| 1 | `create_skill` 是否需要审核 | proposal 模式，必须人工审核     | 直接落盘（当前实现）       |
| 2 | `route_skill` 自动路由     | 删除，改 `load_skill` 工具      | 保留                       |
| 3 | 渲染层（termimad/syntect）  | 删除，纯文本输出                | 抽到 `src/render.rs` 保留  |

未拍板前，TODO 按"默认方案"撰写。需要切换请告知。

---

## 💡 选型参考

- **主思考引擎**: DeepSeek V4 (1M Context)
- **不引入**: 跨会话语义记忆 / 多 LLM provider / plan-execute 框架 / multi-agent 框架 / 浏览器与 MCP（首版）

---

## 历史记录（已废弃，仅供回顾）

阶段一 ~ 阶段四的部分成果将在 **阶段七** 被剥离。这是有意的架构收敛，不是回退。
原始多模态网关代码将通过 git 历史保留，未来如需可重新模块化为外部 plugin（不在核心仓库）。
