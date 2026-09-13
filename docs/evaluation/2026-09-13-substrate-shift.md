# 基底变更：deepseek-flash 具备视觉

> 评估时间：2026-09-13
> 被评估方：SeekCLI `main` @ `ad91414`
> 坐标系：**无**——这不是对照外部清单的评估

---

## 0. 这份文档为什么存在

前三次评估都是同一种形状：拿一份外部坐标系（harness 讲义图3 / dsh / 自进化七条件）
对照，找出「我们缺什么」。

本次不是。**代码没变，基底变了**：模型获得了新能力，于是项目里若干条写死的前提
由真变假。缺口不是「没做」，而是「做对过，但世界动了」。

这类缺口有一个别的评估没有的性质：**它会随时间自己出现**，不需要任何人提需求。
所以值得单独记一篇，并在未来复查 provider 能力时回看。

---

## 1. 实测结论

用 `DEEPSEEK_API_KEY` 直接问 API，不凭记忆：

```text
GET /v1/models          → deepseek-flash, deepseek-v4-pro     （只有两个）
POST /v1/chat/completions  model=deepseek-v4-flash
                        → 响应 model 字段回 "deepseek-flash"   （旧名是别名）
```

视觉能力，用一张模型猜不出来的图验证：

| 输入 | 输出 |
| --- | --- |
| 64×64 PNG，左半蓝、右半黄 | `Blue on the left, yellow on the right.` |

图像开销：16×16 的图令 prompt 从 31 token 涨到 224，64×64 涨到 236。
真实截图会显著更多，**成本口径需要在 L7 复查**。

---

## 2. 由此变假的前提

| 缺口 | 层 | 内容 | 证据 | 级别 |
| --- | --- | --- | --- | --- |
| L0-5 | L0 | 默认模型名过期，且「DeepSeek 是纯文本模型」这条前提已失效 | `src/config.rs:341` 默认 `deepseek-v4-flash`（实为别名）；`~/.seekcli/skills/vision/SKILL.md` 开篇写「DeepSeek V4 是纯文本模型，本身不能"看"图像」 | 功能级 |
| L4-8 | L4 | `Message` 与 `EventPayload` 只能承载纯文本，无法表达多段内容 | `src/api/mod.rs:131` `Simple { content: String }`、`:126` `ToolResponse { content: String }`；`src/session.rs:42` `UserMessage { content: String }` | **架构级** |
| L2-9 | L2 | MCP 返回的图像内容被丢弃 | `src/mcp/protocol.rs:242` 注释「Images … cannot be shown to a text-only model」，输出 `[image content omitted]` | 功能级，**被 L4-8 阻塞** |
| L6-6 | L6 | 无图像输入入口 | 全 crate 无 clipboard / paste 代码（只有 `/copy`） | 功能级，**被 L4-8 阻塞** |

**L4-8 是唯一的架构级项，也是另外两项的前置。** 它要改的是事件日志的 schema：
按 [设计原则 §3](../architecture/design-principles.md) 的「模型可见 = 已记录」，
图像进入请求就必须能从 `events.jsonl` 重建。

---

## 3. 一处需要先澄清的原则冲突

[design-principles §1](../architecture/design-principles.md#1-能力归属) 写着：

> 凡是「客户端预注入」的能力都应改造为 Tool。不再新增 `@xxx` 这类 client-side 解析路径。

阶段七据此剥离了 MinerU / StepFun VLM / Tavily / Jina / GLM Search。那个判断是对的。
但它不能直接套到图像输入上，因为三件事被混在了一起：

| | 违反 §1？ | 为什么 |
| --- | --- | --- |
| 工具返回图像（MCP 截图、读 `.png`） | ❌ | **Agent 自己决定去读的**，正是 Tool Calling 自取 |
| `/paste` 读剪贴板 | ❌ | **剪贴板是用户输入，不是能力**。模型自己碰不到用户的剪贴板，这与用户打一行字同类 |
| 客户端扫描消息里的路径自动塞图 | ✅ | 这才是 `@xxx` 模式：客户端替模型做了决定 |

> **§1 真正排除的是「客户端替模型决定要不要取某项能力」，不是「用户如何把输入交给系统」。**

这条区分必须写进 §1，否则下一个读者会把 `/paste` 当违规删掉——
而那条零摩擦剪贴板流程是明确的用户要求。

---

## 4. 排期

| 阶段 | 内容 | 为什么这样切 |
| --- | --- | --- |
| 四十 | L0-5：纠正模型名与「纯文本」前提 | 纯粹是清理已变假的陈述，不动任何 schema。`vision` skill 现在让 agent 白跑一趟外部 VLM，这个代价每天都在付 |
| 四十一 | L4-8 → 解锁 L2-9 / L6-6 | 架构级，需先写 L4 / L0 层文档设计并修订 §1 |

**四十不等四十一。** 一条会误导 agent 的陈述留着的成本是持续的，
而纠正它不需要等任何架构决定。

---

## 5. 方法学说明

- 模型能力是**实测**的（`/v1/models` + 两次 chat 调用，含一张模型猜不出来的图），
  不是从记忆或文档推断。任何「某模型能不能做某事」的判断都应这样取得。
- 本次未重评 A/B/C/D 四个口径——基底变更不改变那些坐标系下的分数，
  只是新增了四个缺口。
