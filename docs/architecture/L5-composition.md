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
| L5-3 | 无插件 / profile 组合机制 | 取舍级。**阶段五十收窄**：装配的「答案」已落地（`harness_inspect{what:"extensions"}`），不做的是生命周期机制，见 §4.6 |
| L5-4 | 无 hooks | 取舍级 |
| L5-5 | 无 workflow 编排 | 取舍级 |
| ~~L5-7~~ | ~~Skill 的 `allowed_tools` 解析后未消费~~ | **阶段四十二已落地**：在 loop 入口按白名单裁剪 effective 工具面，见 §4.5 | — |
| ~~L5-6~~ | ~~提案闸门只服务 skill~~ | **阶段三十七已落地**：`proposals/{skill,mcp,task}` + `propose` 工具 + `/propose` 闸门。真实验证——起草 → `/propose accept mcp files` → config.toml 追加且原注释一字未动 → 重启后该 server 真的被加载并尝试连接 |

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

### 4.3 提案闸门通用化（L5-6）

> 三评把「选择压力 + 闸门」列为自进化七条件的第 6 条，并判定这是 SeekCLI
> **强于 dsh** 的两处之一——dsh 的四档持久没有升档闸门，保留一个实验要人重写成
> 正式插件。SeekCLI 的 `proposals/` → `accept`/`reject` 已经是真闸门，
> 缺的只是「能走这道闸门的资产只有一种」。

#### 4.3.1 闸门必须和生产者一起做

只泛化闸门而不给新类型一个**生产者**，等于又一个没有用户的抽象——
正是把阶段三十六推后的同一个理由（[design-principles §5.1](design-principles.md#51-三问背后的判据)）。

因此本层同时交付两侧：

- **生产者**：一个 `propose` 工具，模型用它起草任意类型的提案。
- **闸门**：`/propose list | accept | reject`，`/skill` 保留为别名。

触发链也已经存在了：阶段三十五的 `harness_inspect{what:"mcp"}` 让模型看得见
「某个 server 不可用 / 某个能力缺失」，`propose` 是它接下来唯一该做的动作。
**这是「MCP 就是我们的插件格式」这句话第一次真的闭环。**

#### 4.3.2 布局

```text
~/.seekcli/proposals/<type>/<name>…      ← 新家，按类型分目录
~/.seekcli/skills/proposals/             ← 旧家，首见时搬过去（可见一行输出）
```

搬迁风险低：提案本来就是待审的临时物，且是目录 rename。

#### 4.3.3 每类的落地动作与校验

**接受一个坏提案比拒绝一个好提案贵得多**，所以每类都必须能在接受前机械校验。

| 类型 | 落地动作 | 接受前校验 |
| --- | --- | --- |
| `skill` | `proposals/skill/<n>` → `skills/<n>` | 目录内有可解析的 `SKILL.md`；重名拒绝 |
| `mcp` | 向 `config.toml` **追加**一段 `[[mcp]]` | 反序列化成 `McpServerConfig`；server 名不重复 |
| `task` | 写出 `tasks/<n>/TASK.md` | frontmatter 可解析且正文非空（复用 `split_frontmatter`） |

`mcp` 用**纯追加**而不是改写：`toml 0.8` 是 serde 式的，round-trip 会把用户
config.toml 里的注释全部抹掉，而「首次运行生成一份带注释的配置」是本项目
对用户的承诺。TOML 的 array-of-table 允许 `[[mcp]]` 重复出现在文件任何位置，
所以追加既安全又易于人工复核。

#### 4.3.4 `policy` 类型本轮不做——理由

原计划的第四类是策略规则（allow / deny 模式）。它落地需要**就地修改**
已存在的 `[security]` 表，而 TOML 不允许同名表重复出现，所以追加法不成立。
做对它需要 `toml_edit`（新依赖）或把安全配置拆成一个可合并的独立文件——
那是一个**配置架构决定**，不属于提案闸门的范围。

`skill` / `mcp` / `task` 三类按本节完整交付；布局与命令已按「加一类就是加一个
分支」设计好，将来补 `policy` 不需要重构。

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

### 4.5 Skill 工具裁剪（L5-7）✅ 阶段四十二已落地

**这一条是安全级的，因为它是一处语义不对称，不只是一个没做完的功能。**

`allowed_tools` 这个名字在仓库里有两个意思：

| 出现处 | 阶段九起的行为 |
| --- | --- |
| SubAgent 模板（`subagents/registry.rs`） | 真裁剪，经 `registry::filter_by_allowed` |
| Skill frontmatter（`skills.rs`） | **解析、校验、然后丢掉** |

`skills.rs` 自己的注释写着 "phase 12.5 will wire `allowed_tools` into a
per-skill tool whitelist"——那个阶段从未存在。于是同一个字段一边是权限边界，
一边是装饰；写 SKILL.md 的人没有办法从字段名分辨自己在哪一边。

落地形态：

```text
merge_with_skill(内置 + skill 自带 schema)
        + mcp.schemas()
        → narrow_to_skill(effective, allowed)      ← 只能删，不能加
        → policy gate（每次调用，未变）
```

三条不变量，各有单测：

1. **只能删**。skill 声明 `run_shell` 不等于它因此获得 `run_shell`——
   名字必须已经在工具面上。**声明需求不是被授予权限**，
   实际授权仍然只来自宿主策略与用户批准。
2. **匹配不上的名字被点名**，而不是静默忽略。一个 typo 会让作者以为
   自己的 skill 比实际更窄。
3. **裁剪后为空是合法但会被告知**的配置——否则看起来像模型不肯调工具了。

裁剪发生在 MCP 合并**之后**，所以它作用于 effective 工具面而非只作用于内置工具；
`delegate_to_subagent` 拿到的也是裁剪后的集合，因此 **skill 的裁剪无法通过委派绕过**。

顺带修了两处同源的名实不符：`render_skill_md` 此前从 `skill.tools` **反推**
`allowed_tools:`（于是渲染出的 skill 会声明它其实没有裁剪到的工具），
现在以声明的白名单为准；`ProposalStore::read_skill` 此前不带白名单，
于是 eval 闸门衡量的是一个**未裁剪**的版本、落地的却是裁剪版本——
判决因此在谈论一个从不运行的 skill。

### 4.6 扩展装配：做答案，不做平台（L5-3，阶段五十）

路线图原先写的是「把 Skill / MCP / Task 三类资产的安装、启用、禁用、版本收成
同一套声明式装配」。动手时用本仓自己的三问
（[design-principles §5](design-principles.md#5-与-deepseek-harness-的边界)）
重新过了一遍，收窄成一件事。

**先问这套机制是为了回答什么。** 答案是：

> 现在有什么东西在扩展这个 agent，各自从哪来，处于什么状态？

注册表、生命周期、热插拔都是**为了回答这个问题而存在的机械**。
而在单人 CLI 上，机械的成本高于答案的价值。所以：**给答案，不造机械。**

```text
# extensions
  skill doc_parser             v2       installed, not activated   ~/.seekcli/skills/doc_parser/SKILL.md
  skill ielts_writing          -        installed, not activated   ~/.seekcli/skills/ielts_writing/SKILL.md
  mcp   github                 -        enabled but not connected  npx
  task  reminders              -        defined; run by the scheduler  ~/.seekcli/tasks/reminders/TASK.md
Versions are recorded, not enforced: nothing here is hot-swapped mid-run,
and a change lands on the next run.
```

三个细节是刻意的：

- **`state` 是一句话，不是布尔值。** 三类资产的「启用」根本不是同一件事：
  skill 在被激活前什么也不做（所以静息态诚实的说法是 *installed*，不是
  *enabled*）；MCP server 的 `enabled = true` 说的是配置，而**它是否真的连上了
  是另一回事**——两者分开渲染，否则这一行描述的是配置而不是世界；
  task 由 launchd 跑，**这个进程无从确认它到底有没有被调度**，
  所以只说「已定义」。把三者压成一个统一的 lifecycle，需要这些差异不成立。
- **版本被记录，不被强制。** `Skill` 新增 `version` / `source`（兑现 44.2）。
  阶段四十九的闸门能拒绝一个破坏冒烟集的 skill，但「回滚到能用的那个版本」
  要先知道当时跑的是哪个版本——在这件事写下来之前，回滚是靠记性完成的。
- **不热插拔**，而且**在列表里说出来**。一个不说明自己不做什么的清单，
  会被读成它做了。

#### 4.6.1 明确不做，以及判据何时翻转

| 不做 | 理由 |
| --- | --- |
| 统一 enable / disable | skill 的「启用」= 激活（已是显式的、按会话的）；MCP 已有 `enabled`；task 归 launchd。统一它需要三者的差异不成立 |
| 运行中热插拔 | 一次 Run 用一份快照已经是现状（激活的 skill 持有在 `App` 上，MCP 启动时连接），把它形式化成机制不改变行为 |
| Profile / bundle 组合 | design-principles §2 排除项未变；Skill + TASK.md + 项目级 `.seekcli.toml` 深合并已覆盖 |
| 任意 Rust 动态加载 | 判据未变：跨 ABI 不安全，等价需求由 MCP 的进程边界满足 |

**判据何时翻转**，与 §5.1 同一条：**当某一层出现真实的第二个实现时。**
具体到这里——当 `~/.seekcli/` 下装着的扩展多到「哪个 skill 提供了这个工具」
需要查询而不是回忆时，或者当同一个 skill 出现两个需要并存的版本时。
今天是 11 个 skill、0 个 MCP server、1 个 task，一张表就够了。
