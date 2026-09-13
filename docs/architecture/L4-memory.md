# L4 记忆层：会话事件日志 · 压缩 · 状态外部化

> 完成度 **90%**（阶段二十六后）
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
| ⚠️ L4-8 | `Message` 与 `EventPayload` 只能承载纯文本 | `api/mod.rs:131` / `session.rs:42` 均为 `content: String`。**架构级**：按「模型可见 = 已记录」，图像进入请求就必须能从日志重建 |

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

### 4.5 offload 生命周期（L4-6）✅ 阶段二十六 26.4 已落地

- 路径改为 `~/.seekcli/sessions/<id>/blobs/<sha256>.txt`，**随会话归属**。
- 启动时清理 30 天未访问的 blob；同内容天然去重（内容寻址）。

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
