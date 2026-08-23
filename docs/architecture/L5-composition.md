# L5 组合层：SubAgent · Skill · MCP

> 完成度 **65%**（阶段二十八后）｜ 缺口来源：[评估 §3 L5](../evaluation/2026-08-harness-gap-analysis.md#l5-组合层--40)

## 1. 职责边界

把「一个 Agent + 一组工具」组合成可复用、可裁剪、可扩展的能力单元。

三条正交轴：**谁来做**（SubAgent）、**怎么做**（Skill）、**能做什么**（MCP 扩展工具面）。

## 2. 当前实现

| 资产 | 位置 | 说明 |
| --- | --- | --- |
| SubAgent 模板 | `subagents/registry.rs` | `explore`（只读，max_iter=15）/ `general`（含写，max_iter=20）；两者都不含 `invoke_agent` / `create_skill` |
| 深度限制 | `agent/mod.rs::MAX_SUBAGENT_DEPTH = 3` | 循环顶部 bail |
| 工具裁剪 | `registry::filter_by_allowed` | 按模板白名单 |
| Skill 格式 | `skills.rs` | `<name>/SKILL.md`，YAML frontmatter + Markdown body，agentskills.io 兼容 |
| Skill 资产 | `enumerate_skill_assets` | `scripts/` `references/` 清单自动附加进 system_prompt |
| proposal 审核 | `~/.seekcli/skills/proposals/` | `create_skill` 只能起草，`/skill accept` 才生效 |

**proposal 审核流是正确判断，保持**：模型可以提议方法论，但不能自己改自己的行为基线。

## 3. 缺口

| # | 缺口 | 性质 |
| --- | --- | --- |
| ~~L5-1~~ | ~~无 MCP~~ | **阶段二十八已落地** |
| ⚠️ L5-2 | 子代理一次性，无法续跑 / 通信 | 功能级，**未排期**——阶段三十 30.4（后台子代理）已主动推迟并写明理由 |
| L5-3 | 无插件 / profile 组合机制 | 取舍级，见 §5 |
| L5-4 | 无 hooks | 取舍级 |
| L5-5 | 无 workflow 编排 | 取舍级 |

## 4. 目标设计

### 4.1 MCP 客户端（L5-1）✅ 阶段二十八已落地

**这是打开生态的唯一开关。** 接上之后，web / browser / database / GitHub / 文件系统扩展
一整片能力都不用自己写。

配置：

```toml
[[mcp]]
name    = "github"
command = "npx"
args    = ["-y", "@modelcontextprotocol/server-github"]
env     = { GITHUB_TOKEN = "env:GITHUB_TOKEN" }
enabled = true
```

实现要点：

- **只做 stdio transport**，不做 SSE / HTTP。本地 CLI 场景 stdio 覆盖绝大多数 server。
- 启动时并发拉起各 server，`initialize` → `tools/list`，把结果注册进 L2 工具注册表。
- **命名空间**：`mcp__<server>__<tool>`，与内置工具零冲突，也让模型一眼看出来源。
- **单个 server 失败不影响启动**：记一条警告，跳过该 server，其余照常。
  这条比功能本身更重要——外部进程不可靠是常态。
- MCP 工具**默认全部按「非只读」处理**——我们没写这些工具、也无法检查它们做什么，
  只有 server 明确的 `readOnlyHint` 才配得上并发；猜错就是交错写入。
- MCP 工具**走同一条 `ToolDispatcher` 管线**：策略门、超时、审计日志一视同仁。
  > ⚠️ 首次实现在这里翻了车：`policy::is_mutating` 只按内置工具名匹配，
  > `mcp__fs__write_file` 不在任何列表里，于是 `--read-only` 下**真的把文件写出来了**
  > （端到端验证抓到）。修法是把「我们没写的东西」的判定反过来：
  > 除非 server 明确声明只读，否则一律当作会写。同时删掉了不带声明参数的
  > `check` / `is_mutating` 便利重载——那正是会被顺手误用的不安全默认。
- MCP 工具只给主 agent：子代理模板的 `allowed_tools` 是在不知道用户配了什么的前提下
  写的，悄悄放宽会破坏「工具收窄」这个让子代理又便宜又安全的机制。
- 启动耗时上限（默认 10s/server），超时跳过，避免 REPL 启动被拖慢。

依赖选择：优先手写 JSON-RPC over stdio（约 300 行，零新依赖树），
而不是引入完整 MCP SDK——协议面很小，控制权更值钱。

### 4.2 子代理增强（L5-2）

保持 one-shot 语义（**不引入 multi-agent 框架**），只补两件事：

- `invoke_agent(..., background?: bool)`：后台跑，复用 L2 的 `JobRegistry`；
  完成时经 L1 `inject()` 把摘要送进主轴下一轮。
- 子代理的事件写入**独立 session**（L4），主轴事件里只记 `SubagentDelegated { session_id, summary }`。
  这样子代理的完整轨迹可事后回放，主轴上下文只承担摘要成本。

**不做**：`send_message` / `interrupt_agent` / Agent Teams。
那需要一整套 mailbox + roster 状态机，与「纯 ReAct + 类型化 SubAgent 已足够」的原则冲突。

## 5. 明确不做

| 项 | 理由 |
| --- | --- |
| 插件框架 / profile / bundle 组合 | dsh 需要它是因为要支持第三方发行版；单人 Rust CLI 上，编译期组合 + MCP 已覆盖扩展需求，插件框架的复杂度收益比不成立 |
| hooks 协议桥 | 待 MCP 落地后重估——若用户真的需要 hook，多半可以用 MCP server 表达 |
| workflow / ralph 编排 | 纯 ReAct + 类型化 SubAgent 已足够；DAG 编排是另一个产品 |
| 在线自演化 Skill | 见 [design-principles](design-principles.md) |

## 6. 验收标准

- 配置一个 MCP server（如 filesystem）→ `/tools` 能看到 `mcp__filesystem__*`，模型可正常调用。
- 故意把某个 server 的 command 写错 → REPL 正常启动，仅打印一条警告。
- `/plan on` 状态下调用任意 MCP 工具 → 被策略门拒绝。
- `invoke_agent(background=true)` 不阻塞主轴，完成后摘要出现在下一轮上下文。

## 7. 对应路线

阶段二十八（MCP 客户端，P1）、阶段三十（后台子代理随 job 一起落地，P2）。
