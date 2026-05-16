# SeekCLI

**SeekCLI** 是一个基于 **DeepSeek V4** 的本地 CLI Harness Agent。
它把 DeepSeek 的推理能力 + 本地工具调用 + ReAct 闭环包进一个极简的终端 REPL，
让你在 shell 里直接驱动一个真正会"思考 → 用工具 → 观察 → 再思考"的 Agent。

> **架构纲领**: 见 [`AGENT_ARCHITECTURE.md`](./AGENT_ARCHITECTURE.md)
> **演进路线**: 见 [`TODOs.md`](./TODOs.md)

---

## 🎯 设计定位

SeekCLI 选择**做减法**：

- ✅ **DeepSeek V4 单家深度适配** —— 不做多 LLM 路由
- ✅ **Tool Calling 是唯一能力扩展路径** —— 不再有"客户端预注入"
- ✅ **ReAct + 类型化 SubAgent + 策展 Skill** —— 不引入 plan-execute / multi-agent 框架
- ✅ **本地 CLI 即时性** —— 不引入跨会话语义记忆
- ✅ **安全为一等公民** —— 危险命令必须审批，fs 工具受路径白名单约束

不在范围内：浏览器、MCP（首版）、跨会话向量记忆、自演化 skill。

---

## 🌟 核心能力

| 能力              | 说明                                                                     |
| ----------------- | ------------------------------------------------------------------------ |
| ReAct Loop        | 模型在"思考 → tool_call → 观察"循环里自驱，迭代上限保护                    |
| 类型化 SubAgent   | `invoke_agent("explore", ...)` 派发只读探索子任务，仅带摘要回主轴         |
| Tool 调度         | 内置 `read_file / write_file / list_dir / run_shell` 等工具，schema 注入 LLM |
| Skill 策展        | 内置 skill 模板 + `create_skill` proposal（用户审核后生效）              |
| 上下文压缩        | 长会话自动摘要中段，复用 DeepSeek prompt cache                            |
| 安全护栏          | 危险命令拦截 + fs 路径白名单                                              |
| Session 持久化    | `~/.seekcli/sessions/*.json`，`/load` 可断点续接                          |

> 说明：上表是 **目标形态**。当前代码状态见 [`TODOs.md`](./TODOs.md)。
> 部分能力（schema 注入、审批、压缩、类型化 SubAgent）正在阶段六 ~ 阶段十陆续落地。

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
export DEEPSEEK_API_BASE="..."            # 可选，覆盖默认 API endpoint
```

> 阶段七完成后，SeekCLI 不再读取任何其它供应商的 env vars。如需外部能力（搜索 / 抓页 / OCR）请让模型通过 `run_shell` 自取，或等待后续 MCP 工具接入。

---

## ⌨️ 核心交互

### 会话管理
| 指令                    | 说明                              |
| ----------------------- | --------------------------------- |
| `/model [flash\|pro]`   | 切换 DeepSeek 模型                |
| `/thinking [n\|h\|m]`   | 调整思考强度 (None/High/Max)      |
| `/clear`                | 重置当前会话上下文                |
| `/history`              | 查看历史会话列表                  |
| `/load <id>`            | 加载并继续历史会话                |
| `/quit`                 | 退出                              |

### Skill 系统
| 指令               | 说明                              |
| ------------------ | --------------------------------- |
| `/skill list`      | 列出所有可用 skill                |
| `/skill <name>`    | 激活指定 skill                    |
| `/skill review`    | 审核模型起草的 skill proposal（阶段九）|

### 工具能力（自动由模型调用）
模型在 ReAct 循环中可自主调用：
- `read_file / write_file / list_dir` —— 文件系统
- `run_shell` —— 终端命令（危险命令需审批）
- `invoke_agent` —— 派发子任务
- `create_skill` —— 起草新技能 proposal

---

## 📁 目录结构

```
~/.seekcli/
├── sessions/                会话 JSON 记录
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

---

## 🗺️ 路线图概览

| 阶段     | 主题                                | 状态     |
| -------- | ----------------------------------- | -------- |
| 阶段六   | Harness 核心修补                    | ✅ 完成   |
| 阶段七   | 外围资产剥离                        | ✅ 完成   |
| 阶段八   | L3 安全层（审批 + 路径白名单）       | 🚀 下一步 |
| 阶段九   | L5 组合层升级（SubAgent + Skill）   | 📅 计划   |
| 阶段十   | L4 记忆层（压缩 + cache）           | 📅 计划   |
| 阶段十一 | L6 界面瘦身（中断处理 + 状态指示）  | 📅 计划   |

详细任务拆解见 [`TODOs.md`](./TODOs.md)。

---

## 🧠 心智模型速览

SeekCLI 遵循"七层 Harness Agent 架构"：

```
L6  界面层      REPL · CLI 子命令
L5  组合层      SubAgent 模板 · Skill 策展
L4  记忆层      上下文压缩 · prompt cache · 会话
L3  安全层      命令审批 · 路径白名单
L2  边界层      Tool Dispatcher · schema 注册
L1  引擎层      ReAct Loop · 迭代上限 · 中断处理
L0  基底层      LLM Provider · Streaming · 协议解析
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
