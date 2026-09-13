# L0 基底层：LLM Provider · Streaming · 韧性

> 完成度 **100%**（阶段二十二、二十六后）｜ 缺口来源：[评估 §3 L0](../evaluation/2026-08-harness-gap-analysis.md#l0-基底层--70)

## 1. 职责边界

把「一次模型调用」封装成引擎层可消费的流，且**引擎层零协议感知**。

- **属于本层**：wire 协议差异、SSE 解析、tool-call 分片重组、usage 上报、重试 / 超时 / 退避。
- **不属于本层**：决定调用什么模型、调用几次、调用失败后怎么规划（那是 L1）。

## 2. 当前实现

| 文件 | 职责 |
| --- | --- |
| `api/mod.rs` | provider 中立 schema：`Message` / `Tool` / `StreamItem` / `UsageInfo` + `LlmProvider` trait |
| `api/openai.rs` | `/chat/completions` + OpenAI SSE delta 解析 |
| `api/anthropic.rs` | `/messages` + 结构化事件流解析 + schema 双向翻译 |
| `api/resilience.rs` | `Resilient` 重试/退避装饰器 + `IdleTimeout` 流适配器 |
| `config.rs` | `[[provider]]` 端点表 + `resolve_provider` / `resolve_key` |

关键不变量：`Message` / `Tool` 是 provider 中立的，各 provider 负责双向翻译；
引擎只依赖 `StreamItem`。这条抽象是对的，**保持**。

## 3. 缺口

| # | 缺口 | 证据 |
| --- | --- | --- |
| ~~L0-1~~ | ~~provider 未配置化~~ | **阶段二十二已落地** |
| ~~L0-2~~ | ~~无重试 / 退避~~ | **阶段二十二已落地** |
| ~~L0-3~~ | ~~无请求超时 / 流空闲超时~~ | **阶段二十二已落地** |
| ~~L0-4~~ | ~~无 token 计数~~ | **阶段二十六 26.2 已落地**（`api/tokens.rs`） |
| ⚠️ L0-5 | 默认模型名过期；「DeepSeek 是纯文本模型」这条前提已失效 | `config.rs:341` 默认 `deepseek-v4-flash`（实测为别名）；`skills/vision/SKILL.md` 开篇断言模型不能看图，而实测它能 |

## 4. 目标设计

### 4.1 provider 配置化（L0-1）✅ 阶段二十二已落地

`config.toml` 从「选一个写死的 provider」升级为「声明若干 endpoint」：

```toml
[brain]
active = "deepseek-flash"

[[provider]]
name     = "deepseek-flash"
wire     = "openai"                 # openai | anthropic
base_url = "https://api.deepseek.com"
api_key  = "env:DEEPSEEK_API_KEY"   # env:VAR | file:PATH
model    = "deepseek-v4-flash"
```

- `api_key` 用 `env:` / `file:` 前缀，为将来接 keychain 留位置，**不引入新依赖**。
- 构造逻辑从 `App::new` 移到 `api::build_provider(&ProviderConfig) -> Result<Box<dyn LlmProvider>>`。
- **仍不做运行时多模型路由 / 负载均衡**——见 [design-principles](design-principles.md)。

### 4.2 韧性包装（L0-2 / L0-3）✅ 阶段二十二已落地

新增 `api/resilience.rs`，以**装饰器**方式包住任意 `LlmProvider`，与 CostTracker 同一手法
（不污染 `run_agent_loop`）：

```rust
pub struct Resilient { inner: Box<dyn LlmProvider>, policy: RetryPolicy }

pub struct RetryPolicy {
  pub max_attempts: u32,              // 默认 4
  pub base_delay: Duration,           // 默认 500ms，指数退避 + jitter
  pub max_delay: Duration,            // 默认 30s，退避与 Retry-After 的共同上限
  pub request_timeout: Duration,      // 默认 120s，仅到响应头
  pub stream_idle_timeout: Duration,  // 默认 60s，两个 chunk 之间
}
```

jitter 取**确定性**形式（由 attempt 序号推导）而非随机：随机 jitter 会让退避序列不可测试，
而这里 jitter 唯一要达成的效果是「单个客户端的重复重试不完全等周期」。

重试判据：

| 情况 | 处理 |
| --- | --- |
| 连接错误 / 超时 | 重试 |
| HTTP 429 | 重试，优先尊重 `Retry-After` |
| HTTP 5xx | 重试 |
| HTTP 4xx（非 429） | **立即失败**，重试无意义 |
| 流中途断开（无论是否已产出 tool_calls） | **不重试**，交给 L1 的 Error Recovery |

最后一条是关键：重试的安全边界在「有没有已经发生的副作用」，不在「错误码」。

**实现上比设计更保守**：装饰器只重试「返回流之前」的那次调用，一旦开始出字节就永不重试。
判断「这条流有没有已经产出 tool_calls」需要缓冲整条流，而收益只是多救回一小类失败——
不值得。未分类的错误（downcast 不出 `LlmError`）同样按不可重试处理：
重试一个不认识的失败，可能在重复一件不知道是什么的事。

超时分两级，因为它们防的是不同故障：`request_timeout` 只管到响应头
（不能用来限制生成总时长，模型想说多久说多久），`stream_idle_timeout` 管两个 chunk 之间的静默——
一个返回 200 之后就不说话的服务端，和一个很慢的模型在观感上完全一样，
没有这一级，无人值守的 `--run-task` 会永远挂着且既无输出也无报错。

### 4.3 token 计数 seam（L0-4）✅ 阶段二十六 26.2 已落地

```rust
pub trait TokenCounter { fn count(&self, messages: &[Message]) -> usize; }
```

- `tokens::Heuristic`：ASCII 按 4 字节/token，宽字符按 3 字节/token（一个 CJK 字符
  是 3 个 UTF-8 字节、约等于 1 个 token），**零依赖**。每条消息另加 4 token 的封装开销——
  少算这部分会让「很多小消息」的会话看起来比实际便宜。
- 工具调用的 name 与 arguments 也计入：它们同样是模型要付费的上下文，
  忽略会让工具密集的会话被严重低估。
- 压缩阈值（L4）改用 token 而非字节：600_000 **字节** 换成 150_000 **token**。
  旧口径下同一段对话，用中文写会在真实预算约三分之一处就触发压缩。
- 真实 tokenizer 留作后续替换，接口先立住。

### 4.5 模型能力是实测的，不是写死的（L0-5）

#### 4.5.1 问题形状

前四个 L0 缺口都是「我们没写某个机制」。这一个不是：**代码没变，基底变了。**
模型获得视觉能力，于是项目里写死的前提由真变假。

这类缺口会**随时间自己出现**，不需要任何人提需求——所以它的修法不只是改几个字符串，
而是留下一条「下次怎么确认」的路径。

#### 4.5.2 实测方法（复查时照跑）

任何「某模型能不能做某事」的判断都必须这样取得，不能凭记忆或文档：

```sh
# 1. 这个端点到底有哪些模型
curl -sS https://api.deepseek.com/v1/models \
  -H "Authorization: Bearer $DEEPSEEK_API_KEY"

# 2. 一个名字是真模型还是别名：看响应的 model 字段回什么
curl -sS https://api.deepseek.com/v1/chat/completions \
  -H "Authorization: Bearer $DEEPSEEK_API_KEY" -H 'Content-Type: application/json' \
  -d '{"model":"<要查的名字>","messages":[{"role":"user","content":"hi"}],"max_tokens":5}'

# 3. 视觉能力：必须用模型猜不出来的图，否则它从上下文就能蒙对
#    （实测用的是 64×64 左蓝右黄，答 "Blue on the left, yellow on the right."）
```

**第 3 步的陷阱值得记住**：第一次用纯红 16×16 测，模型回「likely red?」——
那可能是从「single color square」这个描述里猜的，不是看见的。
换成猜不出来的双色分割才是有效实验。

2026-09-13 的结果：`/v1/models` 只报 `deepseek-flash` 与 `deepseek-v4-pro`；
`deepseek-v4-flash` 是别名；视觉为真；16×16 的图令 prompt 从 31 token 涨到 224。

#### 4.5.3 本层只做纠正，不做能力

让图像真的进入请求属于 [L4-8 多段内容](L4-memory.md)，是架构级的，另排。
本层只负责**不再陈述假话**：默认模型名、`vision` skill 的前提、
`mcp/protocol.rs` 里把阻塞点归因给模型的注释。

最后一条尤其重要：那句注释写的是「cannot be shown to a text-only model」，
而真正的阻塞点是**我们的 `Message` 还承载不了多段内容**。
理由指错地方，下一个读者就会以为这是模型的限制而绕开它。

## 5. 验收标准

- 断网 / 打 429 时 agent 循环不中断，日志显示退避重试次数。
- `config.toml` 指向任意 OpenAI 兼容端点（如本地 vLLM）可正常对话。
- 服务端不返回数据时，`stream_idle_timeout` 后报错退出而非永久挂起。
- 中英混排长会话的压缩触发点与纯英文一致（token 口径）。

## 6. 对应路线

阶段二十二（P0 韧性 + provider 配置化）、阶段二十六（token 计数随 L4 重构落地）。
