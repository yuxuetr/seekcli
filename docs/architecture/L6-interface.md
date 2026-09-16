# L6 界面层：REPL · CLI · headless 入口

> 完成度 **85%**（阶段四十一 `/paste`、四十七 `verification` 字段后）｜ 缺口来源：[评估 §3 L6](../evaluation/2026-08-harness-gap-analysis.md#l6-界面层--35)

## 1. 职责边界

只做「REPL + 必要状态指示 + 程序化入口」，不抢 Agent 的戏。

任何超过 50 行的渲染美化逻辑必须独立成模块，不得污染 agent loop。

## 2. 当前实现

| 能力 | 位置 |
| --- | --- |
| REPL | `main.rs::App::run`（rustyline） |
| Tab 补全 | `completer.rs::CmdCompleter` |
| slash 命令 | `commands.rs::handle_command`（**18 条**）；`SLASH_COMMANDS` 是唯一清单，`/help` 与 Tab 补全都从它派生，见 §4.5 |
| 状态指示 | prompt 显示 `model (thinking\|plan\|skill) ❯`；`run_shell` >800ms 显示 spinner |
| Ctrl-C | `spawn_interrupt_watcher` + 循环轮询 |
| Ctrl-V 贴图 | `main.rs::PasteKey`（rustyline 条件绑定 → `/paste`），见 §4.4 |
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
| ~~L6-6~~ | ~~无图像输入入口~~ | **阶段四十一已落地**：`/paste` 读剪贴板（`commands.rs`），与 `read_image`、MCP 透传并列为三条图像入口。当时被 L4-8 阻塞，L4-8 解除后一并完成 |

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
{"final":"...","status":"completed","verification":"not_verified",
 "iterations":7,"llm_calls":9,
 "usage":{"prompt_tokens":12000,"completion_tokens":800,"cache_hit_pct":72},
 "cost_cny":0.031}
```

> **2026-09-17 订正。** 上面是实际输出，此前这里写的是阶段二十三的设计草稿，
> 从未与落地对齐：它列了 `session_id` 与 `tools:[{name,count}]` 两个**不存在**的
> 字段，并把 `prompt_tokens` / `completion_tokens` 写成 `prompt` / `completion`。
> 照着这份文档写的 CI 脚本会直接取到 `undefined`。
>
> 订正方向是**改文档而不是补字段**：`-p` 明确不保存会话（见 `run_headless`），
> `session_id` 对它没有意义；`tools` 数组目前没有消费者，而没有消费者的字段是
> 承诺不是灵活性（[AGENTS.md 工程品格](../../AGENTS.md)）。
> `status` 与 `verification` 的分工见 [L7 附](L7-observability.md)。

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

### 4.4 Ctrl+V 贴图

**Cmd+V 做不到，而且永远做不到。** 终端模拟器自己处理 Cmd+V，把剪贴板的*文本*
写进 stdin；剪贴板上是图片时，进程侧什么也收不到。Ctrl+V 则作为控制字符
（0x16）真正抵达应用，所以它是终端程序唯一能绑的贴图键——每一个支持贴图的
终端工具绑的都是它。

代价是占用了 readline 的 `quoted-insert`。这个 REPL 没有插入字面控制字符的用途，
所以是划算的。

三个分支（`paste_command`，纯函数，各有单测）：

| 当前行 | 行为 |
| --- | --- |
| 空 | 插入 `/paste ` —— 截图、Ctrl+V、回车 |
| 已有文字 | 整行替换为 `/paste <文字>`，把已打的字当作 caption |
| 已是 `/paste…` | `Noop` —— 第二次按不会嵌套成 `/paste /paste` |

**它改写命令行，而不是就地抓图。** `/paste` 已经在做「读剪贴板 → 写 blob →
附加」这件事，在两个地方各做一遍正是两者漂移的起点。代价是图片在回车时抓取而非
按键时抓取，期间换了剪贴板则以后者为准——为换取只有一份实现，这个代价可以接受。

**实测**（伪终端驱动，因为 stdin 是管道时 rustyline 根本不走行编辑器，键绑定不生效）：
Ctrl+V 后行缓冲区出现 `/paste `；回车后事件日志里 `UserMessage.images = 1`
（`image/png`），blob 与原文件逐字节相同；256×256 的上红下绿图，模型准确描述了
色块排列。

> 一次值得记下的假警报：同一张图缩到 48×48 时模型说「整张都是蓝的」。
> blob 逐像素核对是正确的（第 5 行红、第 40 行蓝），**是模型对极小图的视觉不可靠，
> 不是管线出错**。换成 256×256 后描述准确。**先验证产物再归咎管线。**

### 4.5 命令清单只有一份（阶段五十一）

`/help`、Tab 补全、`match` 分发曾是**三份各自手维护的清单**，于是它们漂了：
REPL 实际分发 **18** 条命令，Tab 补全只认 **13** 条（`/fork` `/readonly`
`/resume` `/search` `/tools` 补不出来），而本文档还写着 13。

**编译器和行为测试都发现不了这种漂移**——一条补不出来的命令，完整打出来照样能用。

现在 `commands.rs::SLASH_COMMANDS` 是唯一清单：`/help` 渲染它，`completer.rs`
从它取名字。剩下的一份是 `match` 分发，用**源码级校验**对齐——一条测试读
`include_str!("commands.rs")` 抽出所有 `"/x" =>` 分支，双向比对：

- 分发了但不在表里 → 用户按 Tab 补不出来
- 在表里但没分发 → `/help` 宣传了一条不存在的命令

第三条测试守着抽取器本身（若它一条也抽不到，前两条就变成两个空集合相等）。
与 `scripts/check-gap-coverage.py` 同一思路，只是对象从文档换成了代码。

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
