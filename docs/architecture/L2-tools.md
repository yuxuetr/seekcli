# L2 边界层：工具注册与执行管线

> 完成度 **70%**（阶段二十一、二十七后）｜ 缺口来源：[评估 §3 L2](../evaluation/2026-08-harness-gap-analysis.md#l2-边界层--45)

## 1. 职责边界

定义模型能做什么（schema），以及每次调用怎么被执行、被守卫、被观测。

**这是 Agent 能力的上限所在**：L1 再聪明，工具面窄就做不成事。

## 2. 当前实现

11 个工具（`tools/registry.rs::system_tools`）：

| 工具 | 实现 | 只读并发 |
| --- | --- | --- |
| `read_file` | `tools/fs.rs` + `offload.rs` | ✅ |
| `write_file` | `tools/fs.rs` + `path_security` | ❌ |
| `edit_file` | `tools/edit.rs`（L1-L4 模糊匹配链） | ❌ |
| `list_dir` | `tools/fs.rs` | ✅ |
| `glob` | `tools/search.rs` | ✅ |
| `grep` | `tools/search.rs` | ✅ |
| `run_shell` | `tools/shell.rs` + `approval` | ❌（保守排除） |
| `invoke_agent` | 引擎拦截，非 dispatcher | ❌ |
| `create_skill` | `tools/meta.rs` | ❌ |
| `ask_user_question` | `tools/ask.rs` | ❌ |
| `load_skill` | 引擎拦截，非 dispatcher | ❌ |

派发：`tools/mod.rs::ToolDispatcher::execute` 硬编码 `match`，返回 `Result<String>`。

## 3. 缺口

| # | 缺口 | 性质 | 影响 |
| --- | --- | --- | --- |
| L2-1 | **无 MCP** | 架构级 | 第三方无法扩展，只能改 Rust 源码 |
| ~~L2-2~~ | ~~无原生 `glob` / `grep`~~ | — | **已于阶段二十一落地** |
| ~~L2-3~~ | ~~派发无中间件~~ | — | **阶段二十七已落地**（统一管线 + per-tool 超时） |
| ~~L2-4~~ | ~~无结构化 `ToolResult`~~ | — | **阶段二十七已落地**（`ToolKind`，兑现阶段八 8.3） |
| L2-5 | 无后台执行 / job 控制 | 功能级 | 长命令阻塞整个对话 |
| L2-6 | 无持久 PTY | 取舍级 | 明确不做，见 §5 |
| ~~L2-7~~ | ~~无 `ask_user_question`~~ | — | **阶段二十七已落地** |
| L2-8 | 无 `todo_write` | 取舍级 | 已用 PLAN.md / TODO.md 文件约定替代 |

## 4. 目标设计

### 4.1 原生检索工具（L2-2）✅ 阶段二十一已落地

`tools/search.rs`，依赖 `ignore`（复用 ripgrep 的 gitignore 引擎）+ `grep-searcher` / `grep-regex`：

```
glob(pattern, path?)         -> 匹配文件路径列表，按修改时间倒序，上限 200 条
grep(pattern, path?, glob?)  -> 匹配行，格式 `path:line: content`，上限 200 条
```

设计要点：

- **默认尊重 `.gitignore`**，避免把 `target/` `node_modules/` 灌进上下文。
  用 `require_git(false)`：ripgrep 只在 git 仓库内应用 .gitignore，
  而这里的目的是防止构建产物挤占上下文窗口，在没有 `.git` 的普通目录同样成立。
- `hidden(false)` 让 `.github` / `.cargo` 这类正常源码可见，但必须显式
  `filter_entry` 掉 `.git`，否则一次 `glob("*")` 会返回上千个 object 文件。
- 超上限时截断并显式告知「还有 N 条未显示，请缩小范围」——**不静默截断**。
- 两者都是**只读**，进 `is_parallel_readonly` 白名单，可与 `read_file` 并发；
  实际遍历是同步 IO，走 `spawn_blocking`，不阻塞并发批次依赖的运行时。
- 单行输出上限 300 字符：一行压缩后的 bundle 可能有兆字节，
  按行截断能防止一条宽匹配挤掉另外 199 条结果。
- 落地后同步改 `read_file` 的 description，删掉「用 run_shell 配 sed/grep」的引导。

### 4.2 执行管线中间件化（L2-3 / L2-4）✅ 阶段二十七已落地

`ToolDispatcher` 从 `match` 升级为注册表 + 中间件链：

```rust
#[async_trait]
pub trait ToolImpl: Send + Sync {
  fn name(&self) -> &str;
  fn schema(&self) -> Tool;
  async fn execute(&self, args: &Value, cx: &ToolCx) -> ToolResult;
}

pub struct ToolResult { pub kind: ToolKind, pub content: String, pub meta: Value }

pub enum ToolKind { Ok, Denied, Failed, Offloaded, BadArgs }
```

实际落地的管线（顺序固定，不做动态编排）：

```
parse args → policy gate(L3：模式/路径/命令) → deadline → execute → audit
```

保持为一个有序函数而非可组合的中间件栈是刻意的：8 个工具、5 个阶段，插件式栈
只会增加间接层而永远不会被重新配置。真正重要的是**只有一条路径**，任何工具都
绕不开任何一个阶段。

`ToolKind::Denied` **不算失败**：拒绝是一个*决定*，不是故障。把它当失败会让模型
重新规划、绕开策略——而那正是策略存在的意义。Error Recovery 也因此不对拒绝给
恢复建议。

- `ToolKind` 正式兑现阶段八 8.3 的推迟项，取代 `[USER DENIED]` 等字符串前缀约定；
  字符串前缀作为**给模型看的呈现层**保留，但**程序内部不再靠 `contains` 判断**。
- `timeout` 默认 120s，`run_shell` 可由参数覆盖至 600s，超时返回 `Failed` 并附「可用后台模式」提示。

### 4.3 后台任务（L2-5）

```
run_shell(command, background?: bool)   -> background=true 立即返回 job_id
job_list()                              -> 运行中 / 已结束的任务
job_output(job_id, tail?: int)          -> 增量输出
job_kill(job_id)
```

- `JobRegistry` 持有 `HashMap<JobId, JobHandle>`，输出写 `~/.seekcli/jobs/<id>.log`。
- 任务结束时通过 L1 的 `inject()` 在下一轮告知模型，**不打断当前 step**。
- 进程随 REPL 退出而清理，**不做守护进程**。

### 4.4 MCP 客户端（L2-1）

设计见 [L5-composition.md §4.1](L5-composition.md#41-mcp-客户端l5-1)——
MCP 的**接入点**在 L2（工具注册表），但它的**意义**是组合层的生态打开，故放在 L5 描述。

### 4.5 ask_user_question（L2-7）✅ 阶段二十七已落地

```
ask_user_question(question, options?: [{label, description}], multi?: bool)
```

- 交互模式：走 stderr 提示 + rustyline 读取，与 `approval` 的 y/N 同一通道。
- headless 模式（`-p` / `--run-task`）：**直接返回 `Denied` 并说明「非交互环境」**，
  不阻塞、不猜测。这条比功能本身更重要。

## 5. 明确不做

| 项 | 理由 |
| --- | --- |
| 持久 PTY / terminal_* 工具族 | 需要 `node-pty` 级别的跨平台 PTY 管理，复杂度与单人 CLI 不匹配；`run_shell` + 后台 job 覆盖 90% 场景 |
| Code Mode（`run_code`） | 需要内嵌脚本运行时 + 工具桥接，收益在超大工具面时才显现 |
| `todo_write` | PLAN.md / TODO.md 文件约定已覆盖，且天然跨压缩持久 |

## 6. 验收标准

- `grep("fn main", ".")` 直接返回结果，全程不触发审批提示。
- 模型对不存在的路径调 `read_file` → 返回 `ToolKind::Failed` 且带 `[Recovery]` 建议。
- `run_shell("sleep 300", background=true)` 立即返回，`job_output` 能读到增量输出。
- headless 下 `ask_user_question` 返回明确的非交互拒绝，不挂起。

## 7. 对应路线

阶段二十一（glob/grep，P0）、阶段二十七（管线中间件化，P1）、
阶段二十八（MCP，P1）、阶段三十（后台 job，P2）。
