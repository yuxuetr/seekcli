# L4 记忆层：会话事件日志 · 压缩 · 状态外部化

> 完成度 **95%**（阶段四十二增量日志、四十五跨会话记忆、四十七记账事件后）
> 缺口来源：[评估 §3 L4](../evaluation/2026-08-harness-gap-analysis.md#l4-记忆层--45架构级欠账)

## 1. 职责边界

回答三个问题：模型这次能看到什么（上下文）、这次之后还记得什么（持久化）、
上下文超限时丢什么（压缩）。

## 2. 当前实现

| 机制 | 位置 | 说明 |
| --- | --- | --- |
| 阶梯降级压缩 | `agent/compressor.rs` | Stage1 掩码远期 ToolResult（**保留 ToolCall 意图链**）→ Stage2 尾部 Head-Tail 截断 → Stage3 LLM 摘要 |
| 工具大输出卸载 | `tools/offload.rs` | >8K 写 `~/.seekcli/tmp/<hash>.txt`，返回头尾预览 + 路径 |
| 会话事件日志 | `session.rs` | append-only `SessionEvent`；`derive_messages` 是工作集的**唯一**来源 |
| 会话存储 | `history.rs` | `sessions/<id>/{meta.json, events.jsonl, blobs/}`；`/history` 只读 meta |
| 状态外部化 | `agent/prompt.rs::plan_mode_rules` | 引导模型把长程状态写进工作区 PLAN.md / TODO.md |

压缩策略的方向是对的（保留意图链、摘要作为最后一级），**保持**。

## 3. 缺口

| # | 缺口 | 证据 | 性质 |
| --- | --- | --- | --- |
| ~~L4-1~~ | ~~没有 append-only 事件日志~~ | **阶段二十六已落地** | — |
| ~~L4-2~~ | ~~无 checkpoint / fork / resume-at-point~~ | **阶段二十六已落地**（`/resume` `/fork`） | — |
| ~~L4-3~~ | ~~无会话检索~~ | **阶段二十六已落地**（`/search`，扫描而非索引） | — |
| ~~L4-4~~ | ~~无标题生成~~ | **阶段二十六已落地**（首条提示词首行；LLM 生成待评估是否值得一次额外调用） | — |
| ~~L4-5~~ | ~~`list_sessions` 全量读盘~~ | **阶段二十六已落地**（只读 `meta.json`） | — |
| ~~L4-6~~ | ~~offload 无生命周期管理~~ | **阶段二十六 26.4 已落地**（归属会话 + 30 天清理） | — |
| ~~L4-8~~ | ~~`Message` 与 `EventPayload` 只能承载纯文本~~ | `api/mod.rs:131` / `session.rs:42` 均为 `content: String`。**阶段四十一已落地**：三条入口（`/paste`、`read_image`、MCP 透传），事件日志存 blob 引用，过期显式降级。真实验证见 §4.6 |
| ~~L4-10~~ | ~~压缩阈值与模型窗口脱钩~~ | 150K 与 `KEEP_TAIL=8` 是全局常量，而 provider 由配置静态选择、支持两套 wire 协议。**阶段四十二已落地**：`[memory]` 配置 + `Budget`，见 §4.7 | — |
| ~~L4-11~~ | ~~无跨会话领域状态~~ | 学习进度 / 关注标的这类以月计的状态无处安放，而这些对话没有工作目录。**阶段四十五已落地**：分档 Markdown 记忆，见 §4.8 | — |
| ~~L4-12~~ | ~~注入的系统消息不进事件日志~~ | **阶段四十七已落地**：`ContextInjected` 记账事件按 digest 去重、内容存 blob，**不进投影**。见 §4.9 | — |

### 为什么 L4-1 是架构级

dsh 的核心不变量是 **model-visible means logged**：
任何进入模型请求的东西必须能从 append-only 日志重建。
这一条立住之后，fork / resume / 回放 / UI 保真 / 遥测**全部免费派生**。

SeekCLI 现在存的是「压缩之后的当前 messages」。这意味着：

- 压缩把中段对话摘要掉之后，**原始内容永久丢失**（阶段十 10.3 推迟项的真正代价）。
- 无法回到第 7 步换个方向——快照里没有「第 7 步」这个概念。
- 无法回放一次会话来调试 harness 自身（L7-2 的根因也在这里）。

**每晚一个阶段，迁移成本就多一层。**

## 4. 目标设计

### 4.1 事件日志（L4-1）✅ 阶段二十六已落地

会话从「一个 JSON 文件」变成「一个目录」：

```
~/.seekcli/sessions/<id>/
├── meta.json        标题 / 模型 / cost / 时间戳 / 事件数    ← /history 只读这个
└── events.jsonl     append-only，一行一个事件
```

```rust
pub struct SessionEvent { pub seq: u64, pub ts: DateTime<Utc>, pub payload: EventPayload }

pub enum EventPayload {
  UserMessage { content: String },
  AssistantMessage { content: String, reasoning: Option<String>, tool_calls: Vec<ToolCall> },
  ToolResult { call_id: String, kind: ToolKind, content: String },
  SystemPrompt { kind: PromptKind, content: String },   // 内核 / 工作区规约 / skill
  Compaction { replaced: Range<u64>, summary: String }, // 压缩是事件，不是破坏性操作
  SkillActivated { name: String },
  ModeChanged { mode: Mode },
  Interrupted,
  Usage(UsageInfo),
}
```

关键设计：

- **`Compaction` 是一个事件，而不是对历史的改写。**
  投影时遇到它就跳过被替换区间、插入摘要；但原始事件仍在文件里。
  这一条同时解决 L4-1 的信息丢失与 L7-2 的不可回放。
- **投影函数是唯一的上下文来源**：

  ```rust
  pub fn derive_messages(events: &[SessionEvent]) -> Vec<Message>
  ```

  引擎不再直接持有 `Vec<Message>` 作为真相，而是持有事件流 + 投影结果。
- 写入用 append + `flush`，**不做 fsync**（单用户本地 CLI，崩溃丢最后一条可接受）。
  读取时单行损坏只跳过该行并告警，不让一条半写的记录毁掉整个会话。
- 追加消息与追加事件必须**同一个动作**（`engine::log_push`）：一旦某处只写其一，
  「模型可见即已记录」就悄悄不成立了，而不会有任何东西报错。
- 技能切换**不再删除**旧技能的 system 消息：append-only 日志无法也不应撤回事件——
  那个技能在那几轮里确实生效过，抹掉它会让日志与模型真正看到的东西不一致。
  改为追加新的激活事件，靠投影中「更靠后」让新提示词胜出。

### 4.2 派生能力（L4-2 ~ L4-5）✅ 阶段二十六已落地

事件流立住后，以下都是小改动：

| 能力 | 实现 |
| --- | --- |
| `/resume <id>` | 读 events → `derive_messages` → 继续 |
| `/fork <id> [seq]` | 复制 events 前 `seq` 条到新 id |
| `/history` | 只读各目录的 `meta.json`，**O(1) per session**（解决 L4-5） |
| 标题生成 | 首个 `UserMessage` 之后触发一次廉价补全，写 `meta.json`（解决 L4-4） |
| 会话检索 | 首版 `/search <kw>` 直接 grep events.jsonl；**不引入 SQLite**（解决 L4-3） |

### 4.3 迁移 ✅ 阶段二十六已落地

- 启动时检测 `~/.seekcli/sessions/*.json`（旧格式）→ 一次性转成目录格式，原文件改名 `.json.bak`。
- 与阶段十二 `/skill migrate` 同一手法：**可逆、失败回滚、不静默覆盖**。

### 4.4 压缩改造 ✅ 阶段二十六 26.2 已落地

- 阈值从字节改为 token（依赖 [L0 §4.3](L0-llm-substrate.md#43-token-计数-seaml0-4) 的 `TokenCounter`）。
- 压缩分两层，因为它们防的是不同的事：
  - **轮次边界**（`maybe_compact_session`）产出 `Compaction` 事件，摘要进日志并跨轮沿用。
    这是修复所在——工作集每轮都从日志重新投影，此前 stage-3 摘要因此蒸发，
    长会话每一轮都要重新付一次摘要调用。
  - **轮内**（`maybe_compress`）仍作用于工作集，作为单轮膨胀的安全网。
    它的掩码与截断是内容变换、幂等、且每轮重新投影后自动重新施加，不写日志也不丢数据。
- 消息区间 → 事件序号的映射由 `derive_messages_indexed` 提供。没有它，唯一能表达的
  压缩就只有「全部替换」，会连压缩器刻意保留的近期尾部一起丢掉。
- 幂等 marker 逻辑可删除——事件流天然幂等。

### 4.4.1 阈值随模型窗口（L4-10）✅ 阶段四十二已落地

26.2 把阈值从**字节**改成 **token** 是对的，但它留下了同一个错误的下一层：
`COMPRESSION_THRESHOLD_TOKENS = 150_000` 仍然是个**全局常量**，而
`brain.provider` 由配置静态选择、支持两套 wire 协议——**这个常量背后的窗口
逐安装而异**。把 SeekCLI 指向一个窗口小于 150K 的模型，压缩永远不触发，
请求直接因长度失败，而日志里没有任何一行把两者联系起来。

```toml
[memory]
context_window_tokens = 200000   # 你的模型的窗口
compact_at_ratio = 0.75          # 阈值 = 窗口 × 比例
keep_tail_messages = 8
```

三条设计约束：

1. **默认值精确复现旧常量**（200000 × 0.75 = 150000，尾部 8）。这次改动
   给的是可配置性与可见性，**不是行为变更**——什么都不做的人不受影响。
2. **越界比例响亮地夹取，不静默honor**。`compact_at_ratio = 5.0` 若被照单全收，
   等于悄悄关掉压缩——正是本类型要终结的那种不可见失败。合法区间 `0.1..=0.95`：
   0 会每轮都压，1 让阈值坐在窗口上、不给模型留输出空间。`keep_tail` 有下限 2，
   低于 2 会让一次压缩把 `tool_call` 与它的结果劈开。
3. **阈值可被读回它的推导式**。压缩日志行与 `harness_inspect{what:"session"}`
   都打印 `compact_at: 150000 tokens (window 200000 x 0.75)`，
   所以日志里的数字能追回产生它的配置。

顺带补上了一个此前无人走过的路径：**生成的默认配置模板从没有往返测试**。
`[memory]` 引入了模板里的第一个浮点数，一个 TOML 不接受的渲染会静默弄坏
所有人的首次运行。现在有一条测试断言模板解析回来恰好等于 `Config::default()`。

### 4.5 offload 生命周期（L4-6）✅ 阶段二十六 26.4 已落地

- 路径改为 `~/.seekcli/sessions/<id>/blobs/<sha256>.txt`，**随会话归属**。
- 启动时清理 30 天未访问的 blob；同内容天然去重（内容寻址）。

### 4.6 多段内容（L4-8）

> **前置**：[design-principles §1.1](design-principles.md#11-能力与用户输入的分界) 已划清
> 「能力」与「用户输入」的界，否则 `/paste` 会被当成违反 §1 而删掉。

#### 4.6.1 约束：不能扫到 75 个构造点

`Message::Simple.content` 是 `String`，全 crate 有 75 处构造 `Message`、123 处写
`content:`。把它改成 `enum Content { Text, Parts }` 会扫到全部——代价与风险都不对。

**`Message` 不是 wire 类型**，这一点已有先例：`anthropic.rs::build_body` 早就在做
转换（把 system 拉到顶层、合并连续 tool 结果）。openai 侧也已有 `strip_reasoning`
这一步。所以：

- `Message::Simple` 加一个 `#[serde(skip)]` 的 `images` 字段。
- **在 wire 边界拼多段内容**：openai 侧展开成 multipart，anthropic 侧显式报错。

> **落地时修正**：设计初稿写的是「75 处构造点一行不改」，这是错的。
> Rust 的结构体字面量必须写全字段，enum variant 也没有 `..Default::default()`，
> 所以 **23 处 `Message::Simple { … }` 字面量各加了一行 `images: Vec::new()`**。
>
> 这个代价是对的，不该绕：编译器把 23 处**全部**指了出来，
> 没有一处能漏。真正被避免的是另一件事——**把 `content: String` 改成枚举**
> 会波及全部 123 处 `content:` 的读写，那才是不可接受的。

#### 4.6.2 图像不内联进事件日志

```rust
UserMessage {
  content: String,
  #[serde(default, skip_serializing_if = "Vec::is_empty")]
  images: Vec<ImageRef>,          // 只存引用
}
```

两个后果，都是要的：

1. **`#[serde(default)]` 让旧日志原样可读**，不需要版本号与迁移链
   （对比阶段二十六那次是真的换了格式，必须迁移）。
2. **一次截图不会让 `events.jsonl` 膨胀数百 KB**。blob 归属机制已经存在
   （`tools/offload.rs`，按 session 分目录，30 天清扫）。

`derive_messages` 重建时读 blob。**blob 不在了要显式降级**——按
[设计原则 §4](design-principles.md#4-错误处理)「降级优于中断，但绝不静默」：
在该位置留一行「图像已过期（30 天清扫）」的文字占位，而不是静默丢掉、
也不是让 `/resume` 失败。

> **这是「模型可见 = 已记录」的一次边界情形**：日志记的是引用而非字节，
> 所以严格说「可重建」依赖 blob 仍在。写明这一点比假装它无条件成立好。

#### 4.6.3 token 计数会严重低估

`api/tokens.rs` 的启发式按文本长度估。实测：16×16 的图令 prompt 从 31 token
涨到 224，64×64 涨到 236。图像的 token 与字符数无关，继续按文本估会把成本
算低一个量级。**本阶段至少要让它不说谎**——按图数加一个保守常量，
并在层文档里写明这是估计而非精确值。

#### 4.6.4 MCP 图像透传（L2-9）的两个已知事实

**一、这条 wire 接受 `tool` 角色的多段内容。** 2026-09-13 实测：构造
`user → assistant{tool_calls} → tool{[text, image_url]}` 的往返，模型答出了
图里的两种颜色（上蓝下黄，猜不出来的图）。所以 L2-9 不被协议阻塞，
只被我们自己的类型阻塞。

> 注意一个坑：thinking 模式下 assistant 消息必须把 `reasoning_content` 带回，
> 否则 API 直接拒 `invalid_request_error`。

**二、剩下的是一个设计决定，不是体力活。** `ToolDispatcher::execute_with` 的
闭包签名是 `Result<String>`，而**所有工具都必须走这一条路**——「没有任何工具
能绕过策略门 / deadline / 审计」是本仓明确的不变量。

所以图像要从那个闭包里出来，有三条路，各有代价：

| 方案 | 代价 |
| --- | --- |
| 闭包改成返回 `Result<(String, Vec<ImagePart>)>` | 每个内置工具的签名都要改，而它们没有一个会返回图像 |
| `Arc<Mutex<Vec<ImagePart>>>` 侧信道 | 能用，但在一条同步语义的路径上引入共享可变状态，读起来像是有并发 |
| `ToolResult` 加 `images`，由 `execute_with` 的调用方在返回后填 | 图像来源与守门分离，但要保证填充点唯一，否则就有了第二条路径 |

**不要在时间压力下随手选一个。** 选错会造出这个代码库一直避免的那种
「绕过唯一路径」的缝，而它一旦存在就很难再收回。

##### 落地：三个都没选，第四个更干净

停一轮之后找到的：**改闭包的输出类型，而不是改每个工具的签名。**

```rust
pub struct ToolOutput { pub text: String, pub images: Vec<ImagePart> }
impl From<String> for ToolOutput { … }      // 文本工具照旧返回 Result<String>
```

`execute_with` 的闭包收 `Result<ToolOutput>`，内置工具在调用点 `.map(ToolOutput::from)`
一次转换即可，**自身签名一行不改**；MCP 分支直接给出带图的 `ToolOutput`。
没有共享可变状态，没有第二条路径，守门仍然只有一处。

三个原方案各自的代价都被绕开了，而它们本身也因此**不必再评估**——
记在这里是为了说明「停一轮」换来了什么。

**持久化点也只有一处**：`App::push_tool_response`。blob 先落盘、再记事件，
所以这一轮即使中途被打断，「模型可见 = 已记录」依然成立。写 blob 失败时在
文本里留一行可见说明，而不是悄悄返回一个没有图的结果——后者会让模型以为
截图到了。

#### 4.6.5 anthropic wire 本阶段不做，但不静默

默认 provider 是 openai wire（实测 DeepSeek 在该 wire 上支持图像）。
anthropic wire 的图像格式不同（`{"type":"image","source":{"type":"base64",…}}`）。
本阶段只做 openai 侧；anthropic 侧遇到带图消息**显式报错**而不是悄悄丢掉——
悄悄丢会让用户以为模型看过那张图。

## 5. 明确不做

| 项 | 理由 |
| --- | --- |
| 跨会话语义记忆 | 与 CLI 即时性目标背离，见 [design-principles](design-principles.md) |
| SQLite / FTS5 | grep events.jsonl 在单机万级会话下足够；引入数据库是复杂度陡增 |
| `assistant/chunk` 级事件 | dsh 需要它做 Web UI 流式回放；纯终端不需要 |

## 6. 验收标准

- 长会话触发压缩后，`events.jsonl` 里仍能读到被压缩掉的原始内容。
- `/resume <id>` 后模型能正确引用中断前的上下文。
- `/fork <id> 12` 产生新会话，其上下文等于原会话前 12 个事件的投影。
- 1000 个会话时 `/history` 在 100ms 内返回。
- 旧格式 session 启动后自动迁移，`/load` 行为不变。

## 7. 对应路线

阶段二十六（会话事件日志重构，P1，**架构级**）。

### 4.8 跨会话记忆（L4-11）✅ 阶段四十五已落地

推翻排除项的论证见
[design-principles §2.1](design-principles.md#21-为什么推翻跨会话语义记忆阶段四十五)。
这里只记形态。

```text
~/.seekcli/memory/
├── preferences.md     ← 关于用户的规则，作用于所有会话；**只有提案闸门能写**
├── ielts.md           ← 领域状态：事实、可修正、作用域窄；模型直接写
├── cqf.md
└── finance.md
```

**两档的区别是结构性的，不是约定。** `MemoryStore::append` 对 `preferences`
直接 `bail`，`memory` 工具把这类写入转成 `Kind::Memory` 提案。这样
「一次失败后的反思不能自动升级为永久规则」就不是提示词里的请求，而是一条路由。

四项元信息各有归宿，不靠每条记录堆注释：

| | 在哪 |
| --- | --- |
| 来源 | 条目行尾的 `<!-- user · 2026-09-16 -->` |
| 适用范围 | **文件本身**就是范围 |
| 更新时间 | 同来源 |
| 撤销方式 | 文件头写明「删掉这一行」；`harness_inspect{what:"memory"}` 给出目录路径 |

#### 4.8.1 注入策略：偏好内联，领域只给索引

这是**防串味**的核心，不是省 token 的优化。

- **偏好内联**：条数少、已被用户批准、对每个回答都成立。让模型自己去取，
  等于有时它不取。
- **领域只列出名字与条数**：把每个 scope 的正文都注入，等于让雅思的薄弱项
  坐在金融问题的上下文里、让关注标的混进学习会话。模型被告知**存在什么**，
  相关时自己去读。

有单测断言「领域正文不出现在注入内容里」与「`preferences` 不重复列进可读 scope」
（已内联的东西再列一遍，是在邀请一次白费的工具调用）。

#### 4.8.2 仍然不做

向量库、嵌入、语义召回、自动抽取用户画像。检索方式是「读你正在谈的那个主题的
文件」——对单人工具这既够用又可检视。嵌入买到的是模糊召回，代价是一份没人能读、
能审、能手改的语料。

### 4.9 记账事件：ContextInjected 与 ChildRun（阶段四十七）

两个新事件都**不产生消息**。这不是省事，是整件事成立的前提。

#### 4.9.1 ContextInjected（L4-12）

`design-principles.md` §3 说「模型可见 = 已记录」。但 `chat` 的流程是
「从日志投影出 `messages` → `ensure_agent_system_prompt` 往里插系统消息」，
插进去的 `workspace_rules` 与记忆注入**不回流日志**，而它们取决于当时的
`AGENTS.md` 和记忆目录——都无法重建。

**看起来更整齐的做法是错的。** 把注入记成 `SystemPrompt`，它会重新投影；
而记忆随用户的笔记变化，append-only 日志不能撤回旧的那条，于是 prompt 会
**堆积自己的历史快照**。

所以记成**记账事件**：

```jsonc
{"ContextInjected": {"kind": "Memory", "digest": "fnv1a:...", "blob": "…/ctx-….txt"}}
```

- **不进投影**（`produces_message` 是白名单，不含它）。
- **内容进 blob**，与图像同一套内容寻址机制——过去的请求可重建，而产生它的
  实时状态早已变化。
- **按 digest 对同 kind 去重**：记忆注入通常逐轮完全相同，一轮一条会把对话
  埋进记账而什么也没多告诉读者。**变了的 digest 才是有意思的那个**，它拿到
  一条新记录和一个新 blob。
- kernel **不记**：它是编译期常量，"模型看到了什么"不需要记录就能回答。

#### 4.9.2 ChildRun

子代理只把摘要文本交回，于是**一个悄悄撞上迭代上限的委派，在日志里与一个正常
完成的委派完全无法区分**。`ChildRun` 记下模板、结束原因、迭代数、耗时，
并以父级的 `tool_call` id 关联，所以两者**无需猜测即可 join**。

事件由 `delegate_to_subagent` **返回**而不是就地记录，这样它进的是与本轮其余
事件同一个 journal 缓冲，落在「请求委派的 assistant 消息」与「报告结果的
tool result」之间的正确位置。
