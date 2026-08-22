# L0 基底层：LLM Provider · Streaming · 韧性

> 完成度 **90%**（阶段二十二后）｜ 缺口来源：[评估 §3 L0](../evaluation/2026-08-harness-gap-analysis.md#l0-基底层--70)

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
| L0-4 | 无 token 计数 | 压缩阈值用字节数，中英混排时阈值漂移可达数倍（随阶段二十六落地） |

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

### 4.3 token 计数 seam（L0-4）

```rust
pub trait TokenCounter { fn count(&self, messages: &[Message]) -> usize; }
```

- 首版 `HeuristicCounter`：ASCII 按 4 字节/token，CJK 按 1.5 字节/token，**零依赖**。
- 压缩阈值（L4）改用 token 而非字节。
- 真实 tokenizer 留作后续替换，接口先立住。

## 5. 验收标准

- 断网 / 打 429 时 agent 循环不中断，日志显示退避重试次数。
- `config.toml` 指向任意 OpenAI 兼容端点（如本地 vLLM）可正常对话。
- 服务端不返回数据时，`stream_idle_timeout` 后报错退出而非永久挂起。
- 中英混排长会话的压缩触发点与纯英文一致（token 口径）。

## 6. 对应路线

阶段二十二（P0 韧性 + provider 配置化）、阶段二十六（token 计数随 L4 重构落地）。
