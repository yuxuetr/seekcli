# L8 循环层：系统级调度 · Headless Task · Digest

> 完成度 **60%** ｜ 缺口来源：[评估 §3 L8](../evaluation/2026-08-harness-gap-analysis.md#l8-循环层--60)

## 1. 职责边界

**L0-L7 是 Harness（单次调用内的工作台），L8 是 Loop（调用之间的调度）。**

L8 不给 Harness 加新工具或新控制流分支——它只是换一套 prompt，
在一个新的触发时机（cron 而非交互输入）下，复用同一个 Harness 跑一次完整 ReAct 循环。
「该不该通知」「摘要写什么」仍由模型经既有工具自主决定。

## 2. 当前实现

| 机制 | 位置 |
| --- | --- |
| 任务分发 | `tasks.rs::run_task` —— chdir 进 tasks 目录 → `run_headless` → chdir 回原 cwd |
| 任务定义 | `tasks.rs::task_spec(name) -> (prompt, skill_name)` —— **硬编码 match** |
| 状态目录 | `~/.seekcli/tasks/`（可由 `config.toml [tasks] dir` 覆盖），含 `digest/` |
| 状态格式 | `reminders.md` / `todos.md` markdown checkbox + `[notified:...]` 标记 |
| 通知 | 模型经 `run_shell` 调 `osascript display notification` |
| 交互提醒 | `tasks::pending_digest_notice` —— REPL 启动时一天一次 |
| 调度 | `examples/launchd/com.seekcli.reminders.plist`，`StartInterval=900` |

**Task 与 Skill 正交**：Skill（L5）回答「怎么做」，Task（L8）回答「何时做、状态存哪」。
`run_headless(prompt, skill)` 让定时任务复用与 `commands::activate_skill` 完全一致的注入路径。

## 3. 缺口

| # | 缺口 | 影响 |
| --- | --- | --- |
| L8-1 | 任务硬编码在 `task_spec` | 加一个任务要改 Rust 并重编译 |
| L8-2 | 模型无法自己排程 | 取舍级，见 §5 |

## 4. 目标设计

### 4.1 任务声明式化（L8-1）

任务从 Rust `match` 分支变成目录，**格式与 Skill 完全同构**（复用既有 frontmatter parser）：

```
~/.seekcli/tasks/
├── reminders/
│   └── TASK.md          frontmatter: name / description / skill / interval_hint
├── digest/
│   └── <date>.md
├── reminders.md
└── todos.md
```

```markdown
---
name: reminders
description: 检查到期提醒并发系统通知
skill: null
interval_hint: 15m
---

（这里是 prompt 正文，即原 `reminders_prompt()` 的内容）
```

- `seekcli --run-task <name>` 读 `<name>/TASK.md`；找不到就报错列出可用任务。
- `interval_hint` 仅供生成 launchd plist 用，**SeekCLI 自己不做调度**。
- 内置任务（reminders）首次运行时自动写出默认 `TASK.md`，用户可直接改。
- 保持 [design-principles](design-principles.md) 的约束：**L8 不新增 Rust 工具**。

### 4.2 plist 生成辅助

`seekcli task install <name>` 依据 `interval_hint` 生成 plist 到标准输出，
用户自己决定是否 `launchctl load`。**不提供自动安装器**——
往用户的 LaunchAgents 里塞东西应当是显式动作。

## 5. 明确不做

| 项 | 理由 |
| --- | --- |
| `schedule_*` 模型工具 | 让模型给自己排程需要一套持久调度状态机；外部 launchd/cron + 声明式 TASK.md 已覆盖 |
| 常驻守护进程 | 与「一次性调用 + 文件外部化状态」的设计一致性冲突 |
| 跨调用语义记忆 | 状态就是 markdown 文件，可读可改可 git 管理，这是特性不是妥协 |

## 6. 验收标准

- 新增一个任务只需写 `~/.seekcli/tasks/<name>/TASK.md`，无需重编译。
- `seekcli --run-task nonexistent` 报错并列出可用任务名。
- 现有 reminders 任务行为不变（首次自动生成 TASK.md 后等价）。

## 7. 对应路线

阶段三十二（任务声明式化，P2）。
