# AGENTS.md

SeekCLI —— DeepSeek + Tools + Harness Agent 核心，单人维护的本地 CLI Agent（Rust）。

> 本文件被 SeekCLI 自己的 `agent::prompt::workspace_rules` 读取并注入系统提示
> （8KB 上限），所以保持精炼：只写「不看就会做错」的东西。

## 动手前先读

| 想改什么 | 先读 |
| --- | --- |
| 任意一层的设计意图 | `docs/architecture/L<n>-*.md` 的「目标设计」与「明确不做」 |
| 为什么要做这件事 | `docs/evaluation/2026-08-harness-gap-analysis.md`（缺口编号 L0-1 ~ L8-2） |
| 该不该做这件事 | `docs/architecture/design-principles.md` |
| 现在做到哪了 | `TODOs.md` |

**不要凭直觉加功能。** 每层文档都有「明确不做」小节，里面的项是权衡过的，
要推翻先改文档再动代码。

## 代码风格

- **2 空格缩进**（覆盖 rustfmt 默认的 4 空格，见 `.rustfmt.toml`）。
- **禁止 `unwrap()` / `expect()`**——测试、示例、构建脚本除外。
  遇到时先解决根因：这个 `Option` 为什么可能是 `None`？能否从类型设计上消除？
  逻辑上真不可达就用 `unreachable!("具体原因")` 并说明为什么。
- 用 `?` 传播，`match` / `if let` 显式处理，`ok_or_else` / `unwrap_or_default` 兜底。
- 注释写「为什么」，不写「做了什么」。本仓库的注释密度偏高且都在解释权衡，
  跟着这个风格写——尤其是绕过某个坑的代码，把坑本身写清楚。

## 架构约束

- **可观测性走装饰器 / 旁路。** CostTracker 包装 `LlmProvider`；Tracing 在
  engine / registry 边界埋点。**`run_agent_loop` 里不得出现计费、重试、录制代码。**
- **降级优于中断，但绝不静默。** 压缩失败继续跑、offload 写盘失败退回内联，
  每次降级必须有一行可见输出。超上限的结果要显式说「还有 N 条未显示」。
- **`impl App` 可以分块到子模块**（`engine.rs` / `commands.rs` / `benchmark.rs`），
  靠 Rust「子模块可见祖先私有项」规则避免 main.rs 膨胀。
- **给模型看的字符串前缀**（`[USER DENIED]` / `[PATH DENIED]` / `[BAD ARGS]` /
  `[Recovery]`）是呈现层约定。新增时同步更新 `agent::prompt::agent_system_prompt`，
  否则模型不知道该怎么反应。

## 交互约定

- **Ctrl+V 贴图**：截图 → Ctrl+V → 回车。绑定在 `main.rs::PasteKey`，
  展开成 `/paste`。**Cmd+V 永远不会工作**——终端自己吃掉它，只送文本进 stdin。
- **不要引入基于路径的图像输入**（`/paste <路径>`、拖拽路径）。
  零路径管理是这条命令存在的全部理由。

## 工程品格：写实现，不写围绕实现的东西

判据：**新增代码里，删掉之后功能就坏掉的部分应当是主体**；删掉只是「少了一层」的，
本来就不该写。论证与出处见 `design-principles.md` §7。

| 会被拒绝 | 为什么 |
| --- | --- |
| trait / 注册表 / 工厂只有一个实现 | 收益要等第二个实现（design-principles §5.1） |
| 只做转发的包装函数 | 读的人多跳一次，才发现什么也没发生 |
| 没有消费者的配置项 | 是承诺，不是灵活性 |
| 按行数而非真实变化边界拆文件 / 只构造两个分支的大错误枚举 | 分层沿变化边界切 |
| `Display` / `From` / builder 写满，核心还是 `todo!()` | 围着目标转，就是不实现目标 |

- **高内聚、低耦合**：一个模块回答一个问题；模块间靠数据连接——返回值让调用方
  处置，不伸手改对方状态（`delegate_to_subagent` 返回 `EventPayload`）。
- **功能完整**：不接受「框架先搭好，逻辑以后填」，没有逻辑的框架无法被验证。
- **增量优于重写**：先问「这套机制回答什么问题」，只回答那个问题，找最小的真实缝。
- **抑制注解用 `#[expect(lint, reason = "...")]`，不用 `#[allow]`**：过期时它自己报警。
- **「不做」要留可执行的重估条件**（一条命令的退出码）。判据本身也要验：按**收益**
  而非可能性定；造一个该红、一个该绿的输入反向验证；先问数据是谁产生的
  （已删代码留下的旧记录、自己的测试流量）。

## 改动 LLM 交互时要格外小心

这一层的 bug 不会编译失败、不会 panic，只会让模型「宣称完成但什么都没做」。
阶段十九那个 bug 的根因是**消息序列形状分布外**（规划轮后以 assistant 结尾）
加**系统提示自相矛盾**（无工具时仍要求「别叙述、直接调用」）。

因此：

- 改 `messages` 序列形状前，先想清楚它是否破坏 assistant→user/tool→assistant 交替。
- 往系统提示加规则前，检查它在「无工具的规划轮」是否仍然成立。
- 用 `SEEKCLI_TRACE=1` 跑一遍，看决策树里该轮 `tool_calls` 是不是真的非零。
- 改动主循环后跑 `src/loop_tests.rs` 的回放测试；它们断言**副作用**（文件是否真的
  被创建）而非模型说了什么。新增轨迹：`SEEKCLI_RECORD=tests/fixtures/<name> seekcli -p "..."`。
- 概率性的提示词缓解**不够**，要配确定性兜底（见 `strip_fake_tool_syntax`）。

## 提交前

```bash
cargo fmt && cargo clippy --all-targets --all-features --tests -- -D warnings
cargo test --all
cargo deny check          # 依赖公告审计，容易被忽略但 pre-commit 会拦
```

pre-commit 钩子会跑全套（含 `typos`）。**不要用 `--no-verify` 绕过。**

## 提交规范

Conventional Commits，正文用中文：

```
<type>(<scope>): <一句话说明>

[TODOs.md 阶段<n> <n.m>]

<为什么这么改，以及权衡了什么>
```

- type：`feat` / `fix` / `docs` / `refactor` / `test` / `chore` / `perf` / `ci`
- **一个任务 = 一个逻辑提交**，不要把无关改动混在一起。
- **禁止添加 `Co-Authored-By:` 或任何 AI 署名 footer。**
- 完成一项后立刻提交，然后更新 `TODOs.md` 的勾选状态（TODO → DONE，附实际做了什么）。

## 目录速览

```
src/
├── main.rs          App struct + REPL + CLI 入口
├── engine.rs        impl App：ReAct 主循环（改这里最容易出上面说的那类 bug）
├── commands.rs      impl App：slash 命令
├── api/             L0：provider 中立 schema + OpenAI/Anthropic 双 wire
├── agent/           L1/L4：prompt · compressor · reminders · recovery
├── tools/           L2/L3：registry · dispatcher · approval · path_security · offload
├── subagents/       L5：SubAgent 类型注册表
├── observability/   L7：cost · trace · bench
└── tasks.rs         L8：--run-task
```

配置在 `~/.seekcli/config.toml`（**不是**仓库里）；运行时数据全在 `~/.seekcli/`。
