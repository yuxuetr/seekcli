# L6 界面层：REPL · CLI · headless 入口

> 完成度 **75%**（阶段二十、二十三后）｜ 缺口来源：[评估 §3 L6](../evaluation/2026-08-harness-gap-analysis.md#l6-界面层--35)

## 1. 职责边界

只做「REPL + 必要状态指示 + 程序化入口」，不抢 Agent 的戏。

任何超过 50 行的渲染美化逻辑必须独立成模块，不得污染 agent loop。

## 2. 当前实现

| 能力 | 位置 |
| --- | --- |
| REPL | `main.rs::App::run`（rustyline） |
| Tab 补全 | `completer.rs::CmdCompleter` |
| slash 命令 | `commands.rs::handle_command`（13 条） |
| 状态指示 | prompt 显示 `model (thinking\|plan\|skill) ❯`；`run_shell` >800ms 显示 spinner |
| Ctrl-C | `spawn_interrupt_watcher` + 循环轮询 |
| 通用 headless | `-p <prompt>` / stdin 管道 / `--output json` / `--max-iter` / `--read-only` / `--yes` / `--cwd` |
| 特化 headless 入口 | `--bench <suite>` / `--run-task <name>` |
| 输出分流 | `ui.rs`：进度与流式输出走 stderr，结果走 stdout |
| 配置分层 | `config.rs`：用户级 + 项目级 + `$SEEKCLI_CONFIG` |

## 3. 缺口

| # | 缺口 | 影响 | 性质 |
| --- | --- | --- | --- |
| ~~L6-1~~ | ~~无通用 headless 模式~~ | **阶段二十三已落地** | — |
| ~~L6-2~~ | ~~无结构化输出~~ | **阶段二十三已落地** | — |
| L6-3 | 无编辑器 / IDE 集成通道 | 取舍级 | |
| L6-4 | 无 TUI / Web UI | 取舍级，**不追求** | |
| ~~L6-5~~ | ~~配置文件读 CWD~~ | **阶段二十 20.2 已落地** | — |

## 4. 目标设计

### 4.1 通用 headless（L6-1 / L6-2）✅ 阶段二十三已落地

```
seekcli -p "重构 foo.rs 里的错误处理"     # 一次性执行，结果打到 stdout
cat bug_report.md | seekcli -p "分析这个 bug"   # stdin 作为附加上下文
seekcli -p "..." --output json            # 结构化输出
seekcli -p "..." --resume <session-id>    # 在既有会话上继续（依赖 L4）
seekcli -p "..." --max-iter 10 --read-only
```

`--output json` 的形状：

```json
{"session_id":"...","final":"...","iterations":7,
 "usage":{"prompt":12000,"completion":800,"cache_hit_pct":72},
 "cost_cny":0.031,"tools":[{"name":"read_file","count":4}],
 "status":"completed"}
```

退出码语义（供 CI 判断）：

| 码 | 含义 |
| --- | --- |
| 0 | 正常完成 |
| 1 | 运行时错误（API 失败、工具异常） |
| 2 | 达到 `max_iter` 未收敛 |
| 3 | 被策略拒绝且无法继续 |

设计要点：

- headless 下**所有交互式提示自动降级**：审批 `Ask` → `Deny`（除非 `--yes`）。
  **绝不静默挂起** —— 一个停在隐藏 `[y/N]` 上的无人值守任务，和进程卡死无法区分。
  拒绝反而让运行继续，并把模型早已认识的 `[USER DENIED]` 交给它自行调整。
- stdin **只在非 TTY 时读取**：读交互式 stdin 会阻塞等 EOF，正是本阶段要消灭的挂起。
- 输出分流由 `ui.rs` 承担：进度、流式输出、审批提示、重试通知一律 stderr；
  结果只在 stdout 出现一次。REPL 里流式输出**就是**结果，故仍走 stdout——
  这是它需要一个模式开关而非无条件重定向的原因。
- `--bench` / `--run-task` 保留，内部统一走同一条 headless 路径。

### 4.2 配置定位修复（L6-5）✅ 阶段二十 20.2 已落地

```
~/.seekcli/config.toml     用户级（主）
./.seekcli.toml            项目级覆盖（可选，仅覆盖出现的字段）
SEEKCLI_CONFIG=<path>      显式指定，最高优先级
```

- 首次运行在 `~/.seekcli/` 生成带注释的默认配置，**不再往 CWD 写文件**。
- 迁移：检测到 CWD 有 `config.toml` 且 `~/.seekcli/config.toml` 不存在 → 提示用户可迁移，不自动搬。

### 4.3 slash 命令补充

随 L4 落地：`/resume <id>` `/fork <id> [seq]` `/search <kw>`；
随 L2 落地：`/tools`（列出当前生效工具，含 MCP 来源）。

## 5. 明确不做

| 项 | 理由 |
| --- | --- |
| TUI（ratatui 等） | 纯文本 + spinner 已够用；TUI 会把渲染复杂度重新引回来 |
| Web UI | 与「本地 CLI Agent」定位冲突 |
| ACP / JSON-RPC server | 待 `--output json` 落地后重估——多数集成需求它已覆盖 |

## 6. 验收标准

- `seekcli -p "统计仓库有多少个 .rs 文件"` 在无 TTY 环境（如 CI）正常返回并退出 0。
- `echo "..." | seekcli -p "总结"` 能读到 stdin。
- `--output json` 的输出可被 `jq` 直接解析，stdout 里没有混入日志（日志走 stderr）。
- 在任意目录运行 SeekCLI 不再产生 `config.toml`。

## 7. 对应路线

阶段二十（配置定位修复，P0）、阶段二十三（headless 通用化，P0）。
