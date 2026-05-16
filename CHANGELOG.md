# Changelog

All notable changes to this project will be documented in this file. See [conventional commits](https://www.conventionalcommits.org/) for commit guidelines.

---

## [0.1.0] - 2026-04-29

### ✨ Features
- **核心适配**: 深度适配 DeepSeek V4 模型，支持 1M 超长上下文，适配 DashScope API。
- **智能渲染**: 集成 `termimad` 实现 Markdown 的美化输出，渲染效果更接近主流 AI 客户端。
- **语法高亮**: 引入 `syntect` 实现代码块实时着色，支持多语言自动识别与美化展示。
- **快捷指令系统**:
    - 新增 `/copy [idx]` 指令：自动提取回复中的代码块并写入系统剪贴板。
    - 新增 `/thinking` 指令：支持配置 DeepSeek 模型的思考模式（None/High/Max）。
    - 新增 `/history` & `/load` 指令：实现会话的持久化保存与断点续接。
- **智能技能路由 (Skill Router)**:
    - 实现基于 LLM 的意图识别，根据输入自动切换对应的专业技能（如 Translator/FileHelper）。
    - 支持自定义 Skill 配置，允许通过 System Prompt 定义不同的 AI 人设。

### 🛠️ Technical Improvements
- **架构升级**: 迁移至 Rust 2024 Edition，使用全异步（Tokio）架构提升流式响应响应速度。
- **交互优化**: 使用 `rustyline` 优化输入体验，支持历史指令回溯。
- **持久化**: 建立 `~/.seekcli` 标准配置目录，统一管理会话和技能配置。

---

## [unreleased]

### 🎨 UX (Phase 11 — Interface Polish)
- **Ctrl-C 中断**：agent 推理或工具执行中按 Ctrl-C 不再杀进程，而是优雅退出到 REPL。通过 `Arc<AtomicBool>` flag + 后台 `tokio::signal::ctrl_c()` watcher 实现。
- **indicatif spinner**：长 shell 命令（>800ms）显示带 elapsed time 的旋转动画，命令很快时不打扰用户。
- **Rustyline Tab 补全**：实现 `CmdCompleter`，支持 slash 命令、skill 名、proposal 名、model 变体、thinking 模式的自动补全。每次 Tab 重新扫描 skills_dir，新建技能立即可补全。

### 📦 Skills (Phase 12 — agentskills.io Format Compatibility)
- **SKILL.md 目录格式**：新 skill 形态为 `<name>/SKILL.md`（YAML frontmatter + Markdown body），与 Anthropic Agent Skills / agentskills.io / Hermes 生态兼容。
- **scripts/ + references/ 注入**：激活 skill 时，子目录中的脚本和参考文档清单（含一行描述）自动追加到 system prompt，模型即可发现并按需调用。
- **`/skill migrate` 一键迁移**：把 `<name>.json` legacy 格式转成 `<name>/SKILL.md` 目录，原文件备份为 `.json.bak`。
- **手写 YAML 前置元数据 parser**：~80 行，不引入 `serde_yaml` / `gray_matter` 依赖。
- **双格式 loader**：`SkillManager::read_skill_dir` 同时识别 `<name>.json` 和 `<name>/SKILL.md`，向后兼容。
- **create_skill 升级**：模型起草的 proposal 也走 `proposals/<name>/SKILL.md` 目录格式。

### 🧠 Memory (Phase 10 — L4 Memory Layer)
- **上下文压缩**：长会话 messages 超过 600KB 字节时自动触发压缩，保留前导 system message + 最近 8 条尾部，中间段交给 DeepSeek 摘要后用 `[Compressed earlier turns]` 替换。实测 87% 压缩率。
- **prompt cache 验证**：`StreamItem::Usage` 解析 DeepSeek 返回的 `prompt_cache_hit_tokens`，每轮末尾打印 `[Usage] prompt=N (cache hit X%, M miss), completion=K`。实测同 session 内连续问答可达 99% cache hit。

### 🧩 Composition (Phase 9 — L5 Layer)
- **SubAgent 类型化**：`invoke_agent` schema 加 `subagent_type` 枚举（`explore` 只读探索 / `general` 通用子任务），每种 template 自带 system_prompt + allowed_tools 白名单 + max_iter。
- **Skill proposal 审核流**：`create_skill` 不再直接生效，而是写入 `proposals/` 等待 `/skill accept` 审核。新增 `/skill proposals` / `/skill accept` / `/skill reject` REPL 命令。
- **`load_skill` 工具**：让模型在 ReAct 里按需切换 persona，替代阶段七删除的 auto-route 机制。

### 🛡️ Safety (Phase 8 — L3 Layer)
- **危险命令审批**：`tools/approval.rs` 通过 token 匹配识别 `rm -rf /`、`sudo`、`curl|sh`、`dd of=/dev/`、fork bomb、`chmod 777`、`git push --force`、`mkfs.*` 等高危模式；命中时 stderr 弹 `y/N` 等待用户确认，拒绝则返回 `[USER DENIED]` 给模型。
- **fs 路径白名单**：`tools/path_security.rs` 通过 lexical normalize 防止 `write_file` 越权到 cwd 之外，拒绝时返回 `[PATH DENIED]`。
- **安全边界声明**：`AGENT_ARCHITECTURE.md §8` 明确"工具层做语义护栏，不做完整 OS 沙箱"的设计选择，与 Hermes / Claude Code 一致。

### 🐛 Critical Fixes
- **修复 streaming tool-call 参数丢失**：OpenAI 协议下 tool call 分片到达，原实现把每个 delta 单独反序列化导致 arguments 累积失败，引发"模型反复尝试调工具但收到空参数"的死循环。新增 `PartialToolCall` 累加器按 `index` 拼接片段。
- **修复 load_skill 触发 API 400**：原实现在 tool_response 推入前先 push 了 system message，违反 DeepSeek API 严格要求的"assistant{tool_calls} 必须紧接 tool 消息"。改用 `deferred_system_msgs` 队列在 tool batch 处理完毕后追加。

### 🪓 Refactor (Phase 7 — Peripheral Strip)
- **大幅瘦身**：src/ 从 ~2200 行减至 1528 行 (-672)；Cargo.toml 依赖从 20 个减至 14 个。
- **删除多模态网关**：MinerU (PDF/Docx 解析) / StepFun VLM (图像理解) / GLM Web Search / Tavily / Jina Reader 全部移除。Provider 枚举只剩 DeepSeek 隐含。
- **删除外围 slash command**：`/image` `/file` `/web` `/search` `/tavily` 全部移除；`chat()` 中 `@image / @url / @search / @tavily / @pdf` 客户端解析路径取消。
- **删除 auto-route**：`route_skill` 自动技能路由及 `/skill auto` 开关移除。模型应在 ReAct 里自取 skill（阶段九 `load_skill` 工具实现）。
- **删除渲染层**：termimad + syntect 智能渲染框 `┏━━━ 智能渲染视图 ━━━┓` 移除，纯文本输出。`/copy` 用非正则行扫描提取代码块。
- **API 简化**：`Message::Simple.content` 从 `MessageContent` enum 改为 `String`；`ApiClient::new` 签名从 4 参数简化为 1。
- **依赖清理**：移除 base64 / image / mime_guess / termimad / syntect / regex / crossterm / directories；reqwest 移除 multipart feature。
- **环境变量精简**：仅保留 `DEEPSEEK_API_KEY` 与可选 `DEEPSEEK_API_BASE`；不再读取 ZHIPU/STEP/MINERU/TAVILY/JINA/DASHSCOPE 等。

### ✨ Features (Phase 6 — Harness Core)
- **工具 schema 注入**：新增 `src/tools/registry.rs`，所有内置工具以带 JSON schema 的 `Tool` 形式注册并合并 skill tools 下发给 LLM。这是让 ReAct 循环真正运转的关键。
- **Agent 系统提示构建**：新增 `src/agent/{mod,prompt}.rs`，提供 `agent_system_prompt()` / `subagent_preamble()`。每次 chat 入口通过 `ensure_agent_system_prompt` 保证 system message 在头部，最大化 prompt cache 命中率。
- **ReAct 迭代上限**：`MAX_ITER=25` 硬保护；达到上限优雅退出。
- **SubAgent 深度限制 + 工具裁剪**：`MAX_SUBAGENT_DEPTH=3`；子 agent 工具集过滤 `invoke_agent / create_skill` 杜绝递归与越权。

### 🐛 Fixes
- **修复 streaming tool-call 参数丢失**：OpenAI 协议下 tool call 分片到达，原实现把每个 delta 单独反序列化导致 arguments 累积失败，引发"模型反复尝试调工具但收到空参数"的死循环。新增 `PartialToolCall` 累加器按 `index` 拼接片段，在 `finish_reason` / `[DONE]` 时统一刷出完整 ToolCall。

### 📐 Architecture
- **项目重新定位**：从"DeepSeek + 多模态网关"收敛为"DeepSeek + Tools + Harness Agent 核心"。多模态 sensor (StepFun VLM / MinerU / GLM Web Search / Tavily / Jina) 将在阶段七剥离。
- **引入七层架构纲领**：L0 基底 → L1 引擎 → L2 边界 → L3 安全 → L4 记忆 → L5 组合 → L6 界面。详见 `AGENT_ARCHITECTURE.md`。
- **澄清三个心智区分**：模板 vs 实例、ReAct 无独立 Planner、SubAgent 作为运行时上下文压缩。
- **修正 Hermes Agent 对位**：Skill 在主仓库是人工策展 bundle，在线自演化拆到独立仓库；SeekCLI 同步采用"create_skill → proposal → 人工审核"流程。

### 📝 Docs
- 全面重写 `AGENT_ARCHITECTURE.md`，替换原"三板斧"叙述。
- `TODOs.md` 新增阶段六 ~ 阶段十一推进计划，并标记阶段五已知缺口。
- `README.md` 更新定位、能力清单、路线图与心智模型速览。

### Features
- support deepseek chat and r1 model for cli - ([200af21](https://github.com/yuxuetr/rust-template/commit/200af215b4f6973871bc105f16210946e4316392)) - yuxuetr

<!-- generated by git-cliff -->
