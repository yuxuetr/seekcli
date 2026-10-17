# 七评：对照 Claude Code 扩展体系

> 评估时间：2026-10-17（阶段五十七推进完毕后）
> 坐标系：**B 口径（组件全景）**，参照物换成 Claude Code 官方文档的三页——
> [扩展 Claude Code](https://code.claude.com/docs/zh-CN/features-overview)、
> [Claude Code 如何工作](https://code.claude.com/docs/zh-CN/how-claude-code-works)、
> [探索 .claude 目录](https://code.claude.com/docs/zh-CN/claude-directory)
> 过滤器：[design-principles §5](../architecture/design-principles.md#5-与-deepseek-harness-的边界)
> 的三问。那一节是为 dsh 写的，判据原样适用：**参照物是验收清单，不是模仿对象。**

---

## 0. 结论

**循环同构，扩展面窄是有意的；真正的缺口集中在两处，外加两个当场测出的 bug。**

Claude Code 的「gather → act → verify」三阶段，在它自己的文档里也是**模型行为**，
不是结构阶段——「Claude 始终使用工具……再次运行测试以验证」。它的确定性那一半
不在循环里，在 **hooks** 里。所以「SeekCLI 缺 verify 阶段」拆开看，其实是
「SeekCLI 没有任何由 harness 而非模型触发的检查点」。

| 类别 | 项数 | 说明 |
| --- | --- | --- |
| 已对齐 / 有等价物 | 9 | 会话 resume/fork、skills 按需加载、subagent 隔离、MCP、web 工具、headless、设置分层、系统提醒、用户级常驻指令（`preferences` 记忆） |
| **该做** | **2 + 2 bug** | §2：验收命令（Stop 门）、AGENTS.md 超限；§4：audit 日志的两个 bug |
| 有条件再做 | 4 | §3：文件 checkpoint、运行中引导、指令文件向上查找、`/context` |
| 不做 | 8 | §5：plugins / marketplace、output styles、LSP、动态工作流、跨会话消息、远程/IDE 界面、MCP tool search、用户自定义 subagent 文件 |

---

## 1. 逐项对照

证据列只写能复核的东西（文件:行号 / 命令输出），按 [评分口径 §3](scoring-rubric.md) 的约束。

| Claude Code | SeekCLI 现状 | 判定 |
| --- | --- | --- |
| CLAUDE.md：用户级 + 项目 + 祖先目录 + 子目录嵌套 + `@import`；建议 < 200 行 | 只读 cwd 的 `AGENTS.md`，否则 `CLAUDE.md`，首个命中即停，8 KB 截断（`agent/prompt.rs:8-53`） | **§2.2**（超限）+ §3.3（向上查找） |
| `.claude/rules/` 按路径作用域 | 无 | 不做：单文件 8 KB 内够用，前提是 §2.2 先修 |
| 用户级 `~/.claude/CLAUDE.md` | `preferences` 记忆全文注入，写入走提案闸门（`memory.rs:147`） | 等价，且更保守 |
| Skills：描述常驻、按需全文、`/name`、`disable-model-invocation`、`context: fork` | 描述常驻 + `load_skill` / `/skill`；无隐藏开关、无 fork 运行（`skills.rs`、`engine.rs:524-574`） | 对齐。12 个 skill 的索引成本可忽略，开关无消费者 |
| Subagents：用户用文件定义、预载 skills、选模型、fork 对话 | 硬编码 `explore` / `general`（`subagents/registry.rs:105`） | 不做，见 §5 |
| Hooks：生命周期事件触发脚本，「护栏放在 hooks 里」 | 无。L5 §5 排除「hooks 协议桥」 | **§2.1**——排除理由已失效，见下 |
| MCP：stdio/HTTP、作用域覆盖、工具 schema 延迟加载 | 仅 stdio（L5 明确），无延迟加载 | 对齐。本机 `[[mcp]]` 配置数为 **0**，延迟加载没有对象 |
| Plugins / marketplace | 只读清单 `harness_inspect{extensions}` | 不做（design-principles §2 已排除，本评同意） |
| 权限模式：Auto / Manual / Accept edits / Plan；规则作用于任意工具 | `Normal` / `ReadOnly`；规则只管 shell 子串；cwd 内写入**不询问** | 见 §3.1：默认等于 Accept edits，却没有 checkpoint |
| Checkpoints：编辑前快照，`Esc Esc` 回退 | 无 | §3.1 |
| 会话 JSONL、resume、fork | `events.jsonl` append-only、`/resume`、`/fork`（`session.rs:274`） | 对齐 |
| 压缩：先清旧工具输出再摘要；`/compact <焦点>`；`/context` | 阶梯降级压缩（`agent/compressor.rs`）；无 `/compact`、无 `/context` | 压缩对齐；§3.4 |
| 「读过的文件在磁盘上变了」提醒 | 无；`edit_file` 失配时提示重读（`tools/fs.rs:221`） | 不做：失配提示已兜住后果 |
| 中断（Esc）+ 排队消息在工具完成后同轮读入 | 只有 Ctrl-C（`main.rs:507`） | §3.2 |
| 代码智能（LSP） | 无 | 不做，见 §5 |
| Output styles | 无；skill persona 可替代 | 不做 |
| 自动记忆（MEMORY.md 前 200 行 / 25 KB） | 显式 `memory` 工具；无加载上限 | 对齐。现有三个文件共 967 字节，上限无对象 |
| headless `-p`、JSON 输出 | `-p` / `--output json` / 退出码 0-3 / `--run-task` | 对齐；`verification` 恒为 `not_verified`，见 §2.1 |
| 动态工作流、跨会话消息 | 无 | 不做（design-principles §2：DAG 是另一个产品） |

---

## 2. 该做

### 2.1 验收命令：harness 触发的 Stop 门

**问题。** 循环以「本轮无工具调用」为停止信号（阶段五十七已讨论，信号本身成立）。
但停止**没有任何确定性检查**——模型说完了就完了。阶段十九的
「宣称完成但什么都没做」正是从这条缝出来的；`--output json` 的 `verification`
字段在 `-p` 路径上**恒为 `not_verified`**（L7 §47.3），因为 harness 不知道验收条件。

**Claude Code 的答案**是 Stop hook：停止时跑一个外部命令，返回 block 就把理由塞回
循环。文档原话：「CLAUDE.md 或 skill 中的『永远不要……』是请求，不是保证……
如果规则必须每次都成立，将其作为 hook」。

**为什么 L5 的排除不再成立。** L5 §5 写的是「hooks 协议桥：**待 MCP 落地后重估**——
若用户真的需要 hook，多半可以用 MCP server 表达」。两处都失效了：

1. 重估条件早已满足（MCP 于阶段二十八、2026-08-23 落地），近两个月没有人重估——和 `#[allow]` 比理由活得久是
   同一种病，只是发生在文档里。
2. 「用 MCP 表达」在原理上不成立：MCP 工具**由模型决定**调不调，hook 的全部价值
   在于**不由模型决定**。把 hook 换成 MCP，恰好丢掉了唯一要买的那个性质。

**最小形态——不是 hooks 框架，是一个事件。** 三问：

- 问题真实？是。`verification` 字段的存在本身就是项目承认这个缺口；bench runner
  已经证明「按验证命令退出码判定」可用（`observability/bench.rs:43`）。
- 复杂度承担得起？一个配置项 + 停止点一个分支，估计 60-80 行。
- 更轻的等价物？没有。提示词里写「完成前跑测试」是概率性缓解，AGENTS.md 已明确
  「概率性的提示词缓解不够，要配确定性兜底」。

```toml
# ./.seekcli.toml（项目级，已有的配置层）
[verify]
command = "cargo test --quiet"
```

- 模型停止时，若配置了 `command`：执行；退出码 0 → 结束，`verification = passed`。
- 非 0 → 输出尾部作为 `[VERIFY FAILED]` user 消息送回，循环继续——插入点是共享尾部，
  与 doom-loop 提醒、`[Truncated]` 同一处，不破坏交替；`run_agent_loop` 仍只有一个出口。
- 连续失败上限写成常量（不加配置项，没有消费者）；耗尽后 `verification = failed`，
  状态仍如实报告，不伪装成功。
- 前缀 `[VERIFY FAILED]` 同步写进 `agent_system_prompt`。

**不做**通用 hooks（PreToolUse / PostToolUse / SessionStart ...）。护栏那一半已经
编译进 L3（policy / path_security / approval），对单用户而言比外部脚本更确定。
**重估条件**：出现第二个**不同事件**的真实需求（例如「每次编辑后格式化」被要求两次）。
到那时一个事件变两个，才有抽象的第二个实现。

### 2.2 AGENTS.md 超过自己的注入上限

```
$ wc -c AGENTS.md
10740 AGENTS.md          # 上限 8192（agent/prompt.rs:12）
$ head -c 8192 AGENTS.md | tail -1
## 改动 LL
```

截断点正好落在 `## 改动 LLM 交互时要格外小心`。**SeekCLI 在自己仓库里运行时，
模型看不到**：LLM 交互的注意事项、提交前检查、提交规范（含「禁止 Co-Authored-By」）、
目录速览。文件开头自己写着「8KB 上限，所以保持精炼」。

挤占者是 `## 工程品格` 一节（第 49-136 行，约 88 行）。它对**人**很有价值，
但它是长篇论证，不是「不看就会做错」的规则。

Claude Code 的建议与此一致：CLAUDE.md 保持 200 行以下，参考内容移出去。
**修法**：把工程品格压成规则表 + 一句判据，论证移到 `docs/architecture/design-principles.md`
（它本来就是那里的内容）；再加一条测试断言 `AGENTS.md` ≤ 上限——判据可执行，
下次超限当场红，而不是靠下一次评估碰巧发现。

---

## 3. 有条件再做（附可执行的重估条件）

### 3.1 文件 checkpoint

SeekCLI 在 cwd 内写文件**不询问**——等于 Claude Code 的 Accept edits 模式。
但 Claude Code 敢这么默认，是因为编辑可逆（编辑前快照，`Esc Esc` 回退）；
SeekCLI 有前一半没有后一半，在 git 仓库里由 git 兜底，在**非 git 目录**里没有兜底。

材料都在：session 已有 `blobs/` 内容寻址存储。最小形态是 `write_file` / `edit_file`
执行前把原内容存为 blob、记一条事件，`/undo` 还原上一轮改动的文件。

**当前不做**：没有观察到一次丢失。**重估条件**：审计日志里出现
「`write_file` / `edit_file` 执行于非 git 目录」的记录——前提是 §4 先修好审计日志，
否则这个计数被测试流量污染（见 §4）。

### 3.2 运行中引导（steer）

Claude Code 的排队消息「在工具调用完成后立即读取，在同一轮内」——插入点正是
共享尾部，和阶段五十七的 `[Truncated]` 同一处。所以消息**形状**问题已有答案，
上一轮高估了这部分风险。

真正的成本在 **终端输入争用**：循环运行时审批提示（`approval.rs` 的 rustyline）
也要读终端，排队输入必须和它共用一个读取者。阶段五十五已经示范过「两方抢同一个
tty」会怎样失败——子进程继承 tty 后等一个永远不会上屏的回答。这是一次实打实的重构。

**当前不做**。**重估条件**：trace 里「Ctrl-C 中断后 60 秒内发出、且与中断前
同一任务的新 prompt」出现 ≥ 3 次。

### 3.3 指令文件向上查找

在仓库子目录（如 `src/`）里启动，`AGENTS.md` 不会被找到——这是「无法完成的用户动作」，
按评分口径计分。最小形态：从 cwd 向上找，到 `.git` 所在目录或 `$HOME` 为止，约 15 行。
**当前不做的理由**：没有观察到在子目录启动；`--cwd` 已可规避。**重估条件**：
trace 的 run span 里出现 cwd 不含 `AGENTS.md`、而其祖先目录含有的记录。

### 3.4 `/context`

压缩阈值、注入了哪些上下文、各占多少——模型能用 `harness_inspect` 看，用户看不到。
`record_injected_context` 已经记了每次注入的 kind 与 digest，`/context` 只是把它和
token 估算打印出来。**当前不做**：没有用户问过「什么在占上下文」。
**重估条件**：压缩触发次数在真实会话里出现（`harness_inspect` session 视图 > 0）。

---

## 4. 当场测出的两个 bug（与 Claude Code 无关，但阻塞 §3 的判据）

为 §3 找证据时读了 `~/.seekcli/audit.jsonl`，发现它不能用：

| 现象 | 证据 | 根因 |
| --- | --- | --- |
| **测试流量写进用户的真实审计日志** | 2026-09-16 一天 3510 条，cwd 大量是 `$TMPDIR/seekcli-loop-*`（回放测试的 scratch 目录） | `log_path()` 直接读 `$HOME`（`tools/audit.rs:38`），测试无隔离——与早先「测试会话落进真实 history」是同一类 |
| **467 / 7653 行损坏** | `{{""args_bytesargs_bytes""::3939,,...`——两条记录逐字节交织 | `writeln!(f, "{}", entry)` 对无缓冲的 `File` 可能拆成多次 `write`，`O_APPEND` 只保证单次 write 原子（`tools/audit.rs:98`） |

审计日志是安全记录；它同时损坏且被污染，意味着任何基于它的结论都在回答
「测试跑了什么」。**修法**：先序列化成完整一行再单次 `write_all`；测试里把路径
指向临时目录。这两项应排在 §2 之前——它们是 §3 重估条件的数据源。

---

## 5. 不做

| 项 | 三问中哪一问不过 |
| --- | --- |
| Plugins / marketplace | 问 1：单用户，没有分发对象 |
| Output styles | 问 3：skill persona 是更轻的等价物 |
| LSP 代码智能 | 问 2 / 问 3：LSP 客户端是重活；需要时用 MCP 形态的语言服务器桥接，零 Rust 代码 |
| 动态工作流 / 跨会话消息 | 问 1：design-principles §2「DAG 是另一个产品」 |
| 远程控制 / IDE / Slack / Artifact | 问 1：与「本地 CLI」定位冲突 |
| MCP tool search | 问 1：配置的 MCP server 数为 0 |
| 用户文件定义 subagent | 问 1：只有两种类型，且 persona 由 skill 承担。**重估条件**：需要第三种类型时——届时文件定义比再加一个 Rust 常量便宜 |
| `.claude/rules/` 路径作用域 | 问 1：8 KB 单文件在 §2.2 修好后够用 |

---

## 6. 方法学

1. **「待 X 落地后重估」是一条没有执行者的判据。** L5 的 hooks 排除写了重估条件，
   条件满足近两个月没有人回来看。重估条件要么写成可执行的检查
   （`scripts/check-gap-coverage.py` 那种），要么写进 `TODOs.md`「已知仍开放」——
   只写在层文档的「明确不做」表里等于没写。
2. **一个限制条件最该测的是它自己的配置文件。** 8 KB 上限有截断测试，
   但没有人测过仓库自己的 AGENTS.md 是否在上限内。
3. **找证据之前先验证数据源**——这次在用审计日志回答 §3 之前就发现它不可用，
   AGENTS.md「先问数据是谁产生的」又应验一次。
