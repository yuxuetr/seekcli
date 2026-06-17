# SeekCLI

**SeekCLI** 是一个基于 **DeepSeek V4** 的本地 CLI Harness Agent。
它把 DeepSeek 的推理能力 + 本地工具调用 + ReAct 闭环包进一个极简的终端 REPL，
让你在 shell 里直接驱动一个真正会"思考 → 用工具 → 观察 → 再思考"的 Agent。

> **架构纲领**: 见 [`AGENT_ARCHITECTURE.md`](./AGENT_ARCHITECTURE.md)
> **演进路线**: 见 [`TODOs.md`](./TODOs.md)

---

## 🎯 设计定位

SeekCLI 选择**做减法**：

- ✅ **DeepSeek V4 深度适配,双 wire 协议** —— OpenAI / Anthropic 兼容端点经 `LlmProvider` trait 二选一,但不做运行时多模型路由
- ✅ **Tool Calling 是唯一能力扩展路径** —— 不再有"客户端预注入"
- ✅ **ReAct + 类型化 SubAgent + 策展 Skill** —— 不引入 plan-execute / multi-agent 框架
- ✅ **本地 CLI 即时性** —— 不引入跨会话语义记忆
- ✅ **安全为一等公民** —— 危险命令必须审批，fs 工具受路径白名单约束

不在范围内：浏览器、MCP（首版）、跨会话向量记忆、自演化 skill。

---

## 🌟 核心能力

| 能力              | 说明                                                                     |
| ----------------- | ------------------------------------------------------------------------ |
| 双 wire 协议      | `LlmProvider` trait 适配 OpenAI / Anthropic 兼容端点，引擎零感知，config 切换 |
| ReAct Loop        | 模型在"思考 → tool_call → 观察"循环里自驱，迭代上限保护                    |
| Two-Stage ReAct   | 动态触发"谋动分离"——开局/失败时先无工具纯推理，再带工具执行               |
| System Reminders  | 检测重复工具轨迹（死循环），注入 user 消息打断                            |
| Error Recovery    | 工具失败时追加 `[Recovery]` 行动建议，引导模型走排障 SOP                  |
| 只读并发          | 同一轮全为只读工具时并发执行（涉写自动退化串行）                          |
| 类型化 SubAgent   | `invoke_agent("explore", ...)` 派发只读探索子任务，仅带摘要回主轴         |
| Tool 调度         | 内置 `read_file / write_file / list_dir / run_shell` 等工具，schema 注入 LLM |
| 大输出卸载        | 工具输出 > 8K 落盘 `~/.seekcli/tmp/`，仅回首尾预览 + 路径                 |
| 动态 Prompt       | 启动时读工作区 `AGENTS.md` / `CLAUDE.md` 注入项目规约                     |
| Plan Mode         | `/plan` 引导模型把长任务状态外部化到 `PLAN.md` / `TODO.md`                |
| Skill 策展        | 内置 skill 模板 + `create_skill` proposal（用户审核后生效）              |
| 阶梯降级压缩      | 远期 ToolResult 掩码（保留 ToolCall 意图链）+ 工作记忆掐头去尾 + 摘要兜底 |
| 三态安全护栏      | allow / ask / deny 命令分类（可配置）+ fs 路径白名单                      |
| 可观测性          | 每会话 token/CNY 账单 + `SEEKCLI_TRACE=1` 决策树 + `--bench` 跑分         |
| Session 持久化    | `~/.seekcli/sessions/*.json`（含 cost），`/load` 可断点续接              |

---

## 🛠️ 安装与配置

### 环境要求
- Rust 2024 Edition (v1.85+)

### 快速安装
```bash
git clone https://github.com/yuxuetr/seekcli.git
cd seekcli
cargo install --path .
```

### 环境变量
```bash
export DEEPSEEK_API_KEY="your_key"        # 必选
export DEEPSEEK_API_BASE="..."            # 可选，覆盖 OpenAI 兼容 endpoint
export DEEPSEEK_ANTHROPIC_BASE="..."      # 可选，覆盖 Anthropic 兼容 endpoint
```

### LLM Provider（wire 协议）
`config.toml` 的 `[brain] provider` 选择对接协议（同一个 DeepSeek key 通用）：
```toml
[brain]
provider = "openai"     # DeepSeek /chat/completions（默认）
# provider = "anthropic"  # DeepSeek /messages（Anthropic 兼容）
```
两种协议经 `LlmProvider` trait 适配为同一套 provider 中立 schema（`Message`/`Tool`/`StreamItem`），
引擎层（`engine.rs`）只依赖 `StreamItem`，换 provider 不碰一行引擎代码。两条链路均已实测：
benchmark 3/3 PASS，长对话压缩 + trace 正常。

**Reasoning 处理**：思维链是单轮草稿，两个 provider 在**发送请求时**都 strip 掉历史
`reasoning_content`（Anthropic 的无签名 thinking block 会被拒，OpenAI 回放则纯属浪费 token）；
本地 session 仍保留完整 reasoning，`/history` 可回看。

> 阶段七完成后，SeekCLI 不再读取任何其它供应商的 env vars。如需外部能力（搜索 / 抓页 / OCR）请让模型通过 `run_shell` 自取，或等待后续 MCP 工具接入。

---

## ⌨️ 核心交互

### 会话管理
| 指令                    | 说明                              |
| ----------------------- | --------------------------------- |
| `/model [flash\|pro]`   | 切换 DeepSeek 模型                |
| `/thinking [n\|h\|m]`   | 调整思考强度 (None/High/Max)      |
| `/plan [on\|off]`       | 切换 Plan Mode（外部化 PLAN.md/TODO.md）|
| `/clear`                | 重置当前会话上下文（含 cost）     |
| `/history`              | 查看历史会话列表（含 ¥ 估算）     |
| `/load <id>`            | 加载并继续历史会话                |
| `/copy [index]`         | 复制上条回复中的代码块            |
| `/quit`                 | 退出                              |

### Skill 系统
| 指令                    | 说明                              |
| ----------------------- | --------------------------------- |
| `/skill list`           | 列出所有可用 skill                |
| `/skill <name>`         | 激活指定 skill                    |
| `/skill proposals`      | 列出模型起草的 skill proposal     |
| `/skill accept <name>`  | 通过 proposal 转为正式 skill      |
| `/skill reject <name>`  | 丢弃 proposal                     |
| `/skill migrate`        | 旧 `<name>.json` 一键转 `SKILL.md` |

### 工具能力（自动由模型调用）
模型在 ReAct 循环中可自主调用：
- `read_file / write_file / list_dir` —— 文件系统（大输出自动卸载）
- `run_shell` —— 终端命令（三态 allow/ask/deny 护栏）
- `invoke_agent` —— 派发类型化子任务（explore / general）
- `load_skill` —— 会话中激活已保存的 skill
- `create_skill` —— 起草新技能 proposal

### 可观测性与评估
```bash
SEEKCLI_TRACE=1 seekcli                       # 决策树落盘 ~/.seekcli/traces/<id>.json
seekcli --bench examples/benchmarks/basic.json  # Fail-to-Pass 跑分报表
```
每轮对话结束打印 `[Cost]` 账单（token + cache 命中率 + ¥ 估算），并随 session 持久化。

---

## 📁 目录结构

```
~/.seekcli/
├── sessions/                会话 JSON 记录（含 cost 账单）
├── traces/                  SEEKCLI_TRACE=1 时的决策树（<run_id>.json）
├── tmp/                     工具大输出卸载文件
├── bench/                   benchmark 隔离靶机工作区
└── skills/
    ├── <name>/              ← 推荐：agentskills.io 兼容格式
    │   ├── SKILL.md         主文件：YAML frontmatter + Markdown body
    │   ├── scripts/         可选：辅助脚本（模型通过 run_shell 调用）
    │   └── references/      可选：参考文档（模型按需 read_file）
    ├── <name>.json          ← legacy：单文件格式，仍可加载，可一键 /skill migrate
    └── proposals/           模型起草的 skill，待 /skill accept 审核
```

### Skill 格式速览

`~/.seekcli/skills/translator/SKILL.md`:
```markdown
---
name: translator
description: 中英文双向翻译，保留格式与代码片段
allowed_tools:
  - read_file
  - run_shell
---

# Translator

你是 SeekCLI 的翻译助手。

## 规则
- 保留原文的代码块和 Markdown 结构
- 技术术语首次出现给出英文原词
（详见 references/glossary.md）
```

激活 skill 时，`scripts/` 和 `references/` 目录中的文件清单与一行描述会
自动追加到 system prompt，模型即可发现并按需调用 / 阅读。

### Legacy JSON 迁移

如果你已有旧 `<name>.json` 格式，运行：
```
/skill migrate
```
自动把所有 .json 转成 `<name>/SKILL.md` 目录，原文件备份为 `<name>.json.bak`。

### 📦 内置示例 Skills

`examples/skills/` 目录提供了两个**用 bash + curl + 第三方 API** 给
DeepSeek V4 补强能力的 skill 模板：

| Skill | 能力 | 依赖环境变量 | 系统依赖 |
| ----- | ---- | ------------ | -------- |
| `vision` | 调 StepFun VLM 描述剪贴板图 / 任意图片 | `STEP_API_KEY` | macOS osascript、`jq`、`base64`、`file` |
| `doc_parser` | 调 MinerU 把 PDF/Docx/PPTX 解析成 Markdown | `MINERU_API_KEY` | `jq`、`unzip` |

安装到自己的 skill 目录：
```bash
cp -r examples/skills/vision examples/skills/doc_parser ~/.seekcli/skills/
```

启动后激活：
```
/skill vision           # 然后："看看剪贴板里的图"
/skill doc_parser       # 然后："总结 ~/Downloads/paper.pdf"
```

模型会通过 `run_shell` 自动调用 skill 内的脚本，把 VLM/MinerU 的输出
当作视觉/文档证据继续推理。

这套模式可推广 —— 想接 web 搜索、OCR、Python REPL、本地 ollama 等，
按相同结构（`SKILL.md` + `scripts/`）写 bash 脚本即可，**不需要改 Rust
代码**。

---

## 🗺️ 路线图概览

| 阶段     | 主题                                  | 状态     |
| -------- | ------------------------------------- | -------- |
| 阶段六   | Harness 核心修补                      | ✅ 完成   |
| 阶段七   | 外围资产剥离                          | ✅ 完成   |
| 阶段八   | L3 安全层（审批 + 路径白名单）         | ✅ 完成   |
| 阶段九   | L5 组合层（SubAgent 类型化 + Skill）  | ✅ 完成   |
| 阶段十   | L4 记忆层（压缩 + cache）             | ✅ 完成   |
| 阶段十一 | L6 界面（Ctrl-C + spinner + 补全）    | ✅ 完成   |
| 阶段十二 | Skill 格式标准化（SKILL.md 兼容）     | ✅ 完成   |
| 阶段十三 | L1 运行时纠偏 + 只读并发 + 动态 Prompt | ✅ 完成   |
| 阶段十四 | L4 深化（阶梯降级压缩 + 输出卸载）    | ✅ 完成   |
| 阶段十五 | Plan Mode + 状态外部化                | ✅ 完成   |
| 阶段十六 | 三态 allow/ask/deny 权限              | ✅ 完成   |
| 阶段十七 | L7 可观测（Cost + Tracing + Benchmark）| ✅ 完成   |

对照 Harness 全景图（图3）的 12 项组件已全部落地，详见
[`AGENT_ARCHITECTURE.md §4.1`](./AGENT_ARCHITECTURE.md)；任务拆解见 [`TODOs.md`](./TODOs.md)。

---

## 🧠 心智模型速览

SeekCLI 遵循"分层 Harness Agent 架构"：

```
L7  可观测层    Cost Tracker · Tracing · Benchmark
L6  界面层      REPL · CLI 子命令
L5  组合层      SubAgent 模板 · Skill 策展
L4  记忆层      阶梯降级压缩 · PLAN/TODO 外部化 · 会话
L3  安全层      三态命令审批 · 路径白名单
L2  边界层      Tool Dispatcher · schema 注册 · 只读并发
L1  引擎层      ReAct · Two-Stage · System Reminders · Error Recovery
L0  基底层      LlmProvider trait（OpenAI/Anthropic 双适配）· Streaming · 协议解析
```

理解三个关键区分：
1. **模板（持久）vs 实例（短暂）** —— Tool/Skill/SubAgent 类型是预注册的，实例用完即焚
2. **没有独立的 Planner Agent** —— 规划是主 Agent 思考阶段的步骤，不是另起一个 agent
3. **SubAgent = 运行时上下文压缩** —— 隔离子任务，只把摘要带回主轴

详细论述见 [`AGENT_ARCHITECTURE.md`](./AGENT_ARCHITECTURE.md)。

---

## 🤝 贡献与反馈

欢迎提交 Issue 或 PR。新增工具/skill/sub-agent 模板前请先阅读 `AGENT_ARCHITECTURE.md §7 设计原则约束`。

## 📄 开源协议

[MIT License](LICENSE)
