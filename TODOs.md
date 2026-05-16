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

## ✅ 阶段七：外围资产剥离 (Step 2)
*目标：去掉所有非 Harness 资产，回归"纯 CLI Agent"。独立 PR，方便回滚。*

- [x] **7.1 删除多模态 sensor** — Commit `35e524c` (main) + `fd24874` (api)
    - [x] `api.rs` 移除 `Provider::{Zhipu, DashScope, MinerU, StepFun}` 全部分支。
    - [x] `api.rs` 移除 MinerU 相关 5 个结构体 + 6 个 API 方法。
    - [x] `api.rs` 移除 `Message::new_user_image / new_user_file` 与 `MessageContent::Parts` 多模态类型。
    - [x] `main.rs` 移除 `vlm_sensor / doc_sensor / glm_sensor` 字段与初始化。
    - [x] `main.rs` 移除 `analyze_complex_file / paste_image`。
- [x] **7.2 删除外围 slash command**
    - [x] 移除 `/image` `/file` `/web` `/search` `/tavily`。
    - [x] `chat()` 中移除 `@image / @url / @search / @tavily / @pdf` 客户端解析。
    - [x] 模型需要这些能力走 `run_shell("curl ...")` 即可；MCP 方案留待后续。
- [x] **7.3 删除 auto-route**
    - [x] 移除 `route_skill` 方法及其在 REPL 主循环的调用。
    - [x] 移除 `/skill auto` 开关。
    - [ ] `load_skill` 工具留到阶段九 9.3 实现。
- [x] **7.4 渲染层瘦身**
    - [x] 采纳推荐方案：**删除 termimad + syntect**，纯文本输出。
    - [x] `run_agent_loop` 内不再持有任何渲染句柄；`/copy` 用行扫描提取代码块。
- [x] **7.5 依赖清理** — `Cargo.toml` 从 20 个依赖减至 14 个
    - [x] 移除 `base64 / image / mime_guess`（多模态用）。
    - [x] 移除 `termimad / syntect`（渲染用）。
    - [x] 移除 `regex`（已无任何用处）。
    - [x] 移除 `crossterm / directories`（从未实际使用）。
    - [x] 移除 `reqwest` 的 `multipart` feature。

**验收**：
- `api.rs`: 717 → **310 行** ✅（目标 < 250 略超，因保留了完整 streaming 解析）
- `main.rs`: 1107 → **556 行** ✅（目标 < 500 略超，因保留 /history /load /copy 等会话管理命令）
- src/ 总行数：~2200 → 1528 (-672)
- `cargo build / test / clippy --no-deps` 全过
- 外围环境变量 `ZHIPU/STEP/MINERU/TAVILY/JINA/DASHSCOPE_API_KEY` 已从代码与 README 中清除

---

## ✅ 阶段八：L3 安全层
*目标：让 `run_shell` 从"玩具"变"日常可用"。*

- [x] **8.1 危险命令审批** — `src/tools/approval.rs` (130 行 + 6 单测)
    - [x] 拦截规则覆盖：`rm -rf /|~|$HOME|/abs`、`sudo`、`curl|sh / wget|bash`、`dd of=/dev/`、fork bomb (`:(){...}`)、`chmod 777`、`git push --force/-f`、`mkfs.*`。
    - [x] 命中时 stderr 打印命令 + 原因 + `[y/N]` 提示。
    - [x] 用户拒绝时返回 `[USER DENIED]` 前缀字符串给 LLM。
    - [x] `agent::prompt::agent_system_prompt` 增加规范：见到 `[USER DENIED]` / `[PATH DENIED]` 不要重试同一调用。
    - [x] 词法匹配使用 token 边界（whitespace / `;` / `|` / `&`），避免 `pseudosudo` 误伤。
- [x] **8.2 fs 路径白名单** — `src/tools/path_security.rs` (90 行 + 4 单测)
    - [x] `ensure_within_cwd()` 通过 lexical normalize（不走 canonicalize，可处理不存在的目标文件）+ `starts_with` 校验。
    - [x] `write_file` 校验失败返回 `[PATH DENIED]` 前缀。
    - [x] `read_file` / `list_dir` 不加路径限制（read 限制无法阻止 run_shell 越权，徒增体验损失）。
- [x] **8.3 工具结果分类**（轻量实现）
    - [x] 采用**字符串前缀约定**：`[USER DENIED] ...` / `[PATH DENIED] ...` / 正常返回 / `Err(...)` 由 main.rs 包成 `Error executing ...`。
    - [ ] 完整 `ToolResult { kind, content }` 强类型枚举推迟（侵入 dispatcher + 6 处工具 + main.rs 调用点，性价比不高）。

**验收**：
- `cargo test`：11 个测试全过（6 approval + 4 path_security + 1 skill）
- `cargo clippy --no-deps`：零告警
- 实战测试待用户验证：让模型尝试 `rm -rf ~/foo` 应弹出审批；让模型尝试 `write_file("/etc/foo", ...)` 应被路径白名单拒绝。

---

## ✅ 阶段九：L5 组合层升级
*目标：SubAgent 类型化，Skill 重定位为 proposal 审核流程。*

- [x] **9.1 SubAgent 模板注册表** — Commit `4cccfff`
    - [x] 新增 `src/subagents/{mod,registry}.rs`，`SubAgentTemplate { name, description, system_prompt, allowed_tools, max_iter }`。
    - [x] 内置 `explore`（只读，max_iter=15）和 `general`（带 write_file，max_iter=20）。两者都不含 `invoke_agent / create_skill`。
    - [x] `invoke_agent` schema 加 `subagent_type: enum["explore","general"]` 必选字段。
    - [x] 派发时 `tools::registry::filter_by_allowed` 按白名单裁工具；`max_iter` 跟随模板。
    - [x] 删除孤儿 `filter_for_subagent` / `subagent_preamble`。
- [x] **9.2 Skill proposal 审核机制**
    - [x] `tools::meta::create_skill` 改写到 `~/.seekcli/skills/proposals/`。
    - [x] `SkillManager::{list_proposals, accept_proposal, reject_proposal}` 增加。
    - [x] REPL：`/skill proposals` / `/skill accept <name>` / `/skill reject <name>`。
    - [x] `accept_proposal` 检查目标 skill 不存在才允许 rename，避免静默覆盖。
    - [x] `agent_system_prompt` 明确：create_skill 是 proposal，不是激活；指示模型告诉用户去 review。
- [x] **9.3 Skill 加载工具化**
    - [x] `load_skill` 工具 schema 加入 system_tools。
    - [x] main.rs::run_agent_loop 拦截 `load_skill`（与 `invoke_agent` 同类），把 skill 的 system_prompt 推入 messages 并 set self.current_skill。
    - [x] 子 agent (depth > 0) 调用 `load_skill` 返回 `[ERROR] restricted to the main agent`，杜绝持久化副作用。

**验收**：
- `cargo test`：15 个测试全过
- `cargo clippy --no-deps`：零告警
- 实战测试待用户验证：
  - 让模型用 `invoke_agent("explore", ...)` 派发只读探索 → 应正常返回摘要
  - 让模型尝试 `create_skill` → proposal 落到 `proposals/`，`/skill proposals` 能看到
  - `/skill accept <name>` 后 → `/skill list` 应出现新 skill
  - 让模型在对话中 `load_skill("translator")` → 后续回复应转译风格

---

## ✅ 阶段十：L4 记忆层
*目标：让长会话不爆 token，命中 prompt cache。*

- [x] **10.1 上下文压缩** — `src/agent/compressor.rs` (~150 行 + 2 单测)
    - [x] `maybe_compress(client, model, &mut messages)` 主入口。
    - [x] 阈值 `COMPRESSION_THRESHOLD_BYTES = 600_000` 字节（约 150K~300K token，取决于中英占比）。
    - [x] 策略：保留**所有前导 system message**（agent prompt + 可选 skill prompt，cache 前缀稳定）+ 最近 8 条尾部消息；中间段用 DeepSeek 自己摘要。
    - [x] 摘要以 `[Compressed earlier turns]\n\n<summary>` 作为新 system message 插入。
    - [x] `run_agent_loop` 仅在 `depth == 0`（主 agent）每轮顶部触发；子 agent 跳过。
    - [x] 压缩失败时降级为日志告警 + 继续原 messages，不中断对话。
- [x] **10.2 prompt cache 验证**
    - [x] `api::StreamItem::Usage(UsageInfo)` 新增枚举变体。
    - [x] SSE parser 识别 `usage` 字段（独立于 choices 块出现）。
    - [x] `run_agent_loop` 每轮末尾打印 `[Usage] prompt=N (cache hit X%, M miss), completion=K`。
    - [x] 系统提示自阶段六起即固定（`agent::prompt::agent_system_prompt`），cache 前缀稳定。
- [ ] **10.3 Session raw/compressed 双存档** — **推迟**
    - 评估后认为侵入度过高：history.rs 需要双路径写入 + /load 需要语义判断。
    - 当前 session JSON 已是"压缩后状态"快照，足以满足 /load 续接。
    - 若未来需要审计原始 trace，可作为独立小阶段补做。

**验收**：
- `cargo test`: 17 通过（15 旧 + 2 新 compressor 单测）
- `cargo clippy --no-deps`: 零告警
- 实战测试待用户验证：长对话触发压缩后日志显示压缩率；多轮交互后 `[Usage]` 行 cache hit% 应递增（DeepSeek 缓存前缀生效）。

---

## ✅ 阶段十二：Skill 存储格式标准化（agentskills.io 兼容）
*目标：从 `<name>.json` 单文件迁移到 `<name>/SKILL.md` 目录形式，
兼容 Anthropic Agent Skills / agentskills.io / Hermes 生态。*

- [x] **12.1 SKILL.md 格式定义** — Commit `3169e9a`
    - [x] 主文件 `<name>/SKILL.md`，YAML frontmatter + Markdown body。
    - [x] frontmatter 支持 `name` / `description` / `allowed_tools` / `version`。
    - [x] body 即 system_prompt，模型可直接写 Markdown 无需 escape。
    - [x] 目录支持 `scripts/` 与 `references/` 子目录（C3 扫描注入）。
- [x] **12.2 Parser 与依赖** — Commit `3169e9a`
    - [x] **无新依赖**：手写 ~80 行 YAML frontmatter parser。
    - [x] `read_skill_dir` 同时识别 `<name>.json` 与 `<name>/SKILL.md`。
    - [x] 向后兼容：legacy JSON skill 仍可加载。
    - [x] 9 个单测覆盖 parser 各类边界。
- [x] **12.3 migrate 工具** — Commit `aa4d046`
    - [x] REPL 命令 `/skill migrate` 完成。
    - [x] 失败时回滚已建目录，不留半残状态。
    - [x] 原 .json 自动备份为 `.json.bak`，可逆。
- [x] **12.4 create_skill 工具更新** — Commit `aa4d046`
    - [x] proposal 写入 `proposals/<name>/SKILL.md`。
    - [x] 同名冲突检查覆盖 .json 与 dir 两种形式。
    - [x] 模型迭代同名 proposal 时自动覆盖。
    - [x] accept/reject_proposal 双格式支持。
    - [x] agent_system_prompt 更新 create_skill 描述。
- [x] **12.5 scripts / references 支持** — Commit (本 commit C3)
    - [x] `enumerate_skill_assets(skill_dir)` 扫描子目录。
    - [x] 加载 skill 时把脚本/参考清单（含一行描述）自动追加到 system_prompt。
    - [x] 描述自动从首行注释/标题提取，跳过 shebang。
    - [x] 3 个新单测覆盖空目录、混合资产、SKILL.md + scripts 端到端。
- [x] **12.6 文档对齐**
    - [x] `AGENT_ARCHITECTURE.md §3` Hermes 对比表更新 Skill 来源。
    - [x] `README.md` 加 SKILL.md 格式示例 + migrate 指引。

**验收**：
- cargo test: 30 → 33 通过
- cargo clippy --no-deps: 零告警
- 实战测试待用户验证：`/skill migrate` 把现有 5 个 JSON skill 一键迁移；
  创建带 `scripts/` 的 skill，激活后 system_prompt 含资产清单。

---

## ✅ 阶段十一：L6 界面瘦身
*目标：让界面层只做"REPL + 必要状态指示"，不抢 Agent 戏。*

- [x] **11.1 渲染解耦** — 阶段七已删除 MadSkin/syntect，N/A
- [x] **Ctrl-C 中断**（阶段六遗留）— Commit `4c19a5e`
    - [x] `App.interrupt: Arc<AtomicBool>` + `spawn_interrupt_watcher` 后台任务
    - [x] `run_agent_loop` 每轮顶部 + stream 消费时 check
    - [x] `chat()` 入口重置 flag，避免 readline Ctrl-C 误穿透
- [x] **11.2 状态指示** — Commit `4c19a5e`
    - [x] `indicatif = "0.17"` 依赖
    - [x] `run_shell` 长命令（>800ms）显示 spinner，含 elapsed_precise + 命令预览
    - [x] 命令快完成时 spinner 静默退出（不打扰用户）
    - [x] `deny.toml` 加 `RUSTSEC-2025-0119` ignore
- [x] **11.3 命令补全** — Commit (本 C2)
    - [x] `CmdCompleter` 实现 Completer + Helper + Hinter + Highlighter + Validator
    - [x] Tab 后 `/` → 列出 10 个 slash 命令
    - [x] Tab 后 `/skill ` → 列出 5 个 subcommand + 所有 skill 名
    - [x] Tab 后 `/skill accept ` / `/skill reject ` → 列出 proposal 名
    - [x] Tab 后 `/model ` → `flash` / `pro`
    - [x] Tab 后 `/thinking ` → `n` / `h` / `m`
    - [x] 每次 Tab 重新扫描 skills_dir，新建 skill/proposal 立即可补全

**验收**：
- cargo test: 33 通过
- cargo clippy --no-deps: 零告警
- 实战测试待用户验证：
  - REPL 输入 `/sk` 按 Tab → 补全成 `/skill`
  - 输入 `/skill ` 按 Tab → 列出所有 skill 名
  - 长 shell 命令期间能看到旋转动画
  - Ctrl-C 在 agent 推理时优雅退出回到 REPL

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
