# L3 安全层：审批 · 路径策略 · 模式门禁

> 完成度 **50%，且边界不自洽** ｜ 缺口来源：[评估 §3 L3](../evaluation/2026-08-harness-gap-analysis.md#l3-安全层--50且边界不一致)
> 安全模型的**范围声明**（做什么 / 不做什么 / 用户责任）见 [security-model.md](security-model.md)。

## 1. 职责边界

在工具结构允许的范围内提供**常识性护栏**，让 `run_shell` 从玩具变成日常可用。

**不是** OS 级隔离。这是主动取舍，不是欠账。

## 2. 当前实现

| 机制 | 位置 | 覆盖 |
| --- | --- | --- |
| 三态命令审批 | `tools/approval.rs::classify -> Decision{Allow,Ask,Deny}` | `run_shell` |
| 优先级 | user deny > 内置 deny > user allow > 内置 ask > allow | |
| 写路径白名单 | `tools/path_security.rs::ensure_within_cwd` | `write_file`（`fs.rs:28`）、`edit_file`（`fs.rs:59`） |
| 用户可配 | `config.toml [security] allow/deny` 子串列表 | |

## 3. 缺口

| # | 缺口 | 证据 | 性质 |
| --- | --- | --- | --- |
| L3-1 | **写受限、shell 不受限** | 路径检查只挂在两个 fs 工具上；`sh -c "cat > ~/.ssh/x"` 不过路径检查 | **边界不自洽** |
| L3-2 | **Plan Mode 只是 prompt** | `engine.rs:339` 仅注入 `plan_mode_rules()`，不阻断写工具 | **名不副实** |
| L3-3 | 审批是子串匹配 | `matches_any` 大小写不敏感子串 | 可绕过 |
| L3-4 | 无审计日志 | 无 | 事后不可追溯 |
| L3-5 | 无进程沙箱 | 无 | **取舍级**，见 security-model |

> L3-1 与 L3-2 的共同点：**声明的边界与实际执行的边界不一致**。
> 这比「边界画得小」危险得多——用户会按声明去信任它。优先级因此高于功能性缺口。

## 4. 目标设计

### 4.1 统一策略门（L3-1 / L3-2）

三个正交维度合成一次判定，所有工具**走同一个门**：

```rust
pub struct PolicyGate { mode: Mode, workspace: PathBuf, rules: Rules }

pub enum Mode {
  Normal,     // 默认
  Plan,       // /plan on：拒绝一切写与副作用
  ReadOnly,   // --read-only：同 Plan，但不注入 plan prompt
}

pub enum Verdict { Allow, Ask(String), Deny(String) }

impl PolicyGate {
  pub fn check(&self, tool: &str, args: &Value) -> Verdict;
}
```

判定顺序：

1. **模式门**：`Plan` / `ReadOnly` 下，`write_file` / `edit_file` / `create_skill` 直接 `Deny`；
   `run_shell` 降级为「仅 allow 名单内的只读命令」。
   —— 这让 Plan Mode 从「提示模型别写」变成「写不了」。
2. **路径门**：对声明了路径参数的工具做 `ensure_within_cwd`。
   **`run_shell` 新增轻量路径提取**：扫描命令中的绝对路径与 `~` 展开，
   任一指向工作区外 + 该命令含写意图（`>` `>>` `tee` `cp` `mv` `rm` `install`）→ `Ask`。
3. **命令门**：`run_shell` 走现有 `classify`。

### 4.2 命令解析升级（L3-3）

`matches_any` 的裸子串换成 `shlex` 分词后按 token 匹配：

- 消除 `pseudosudo` 类误伤（现已用 token 边界缓解，但仍是字符串层面）。
- 识别 `$(...)` / 反引号 / `;` / `&&` 分隔的**子命令**，逐条判定，取最严结果。
- **明确不做** shell AST 完整解析——见 security-model §8.2 的既有取舍。
  目标是「显著提高绕过成本」，不是「不可绕过」。

### 4.3 审计日志（L3-4）

`~/.seekcli/audit.jsonl`，每次工具调用一行：

```json
{"ts":"...","session":"...","tool":"run_shell","verdict":"ask","approved":true,
 "args_digest":"sha256:...","cwd":"/path","duration_ms":120}
```

- 记 `args_digest` 而非原文，避免把密钥写进日志；`run_shell` 例外记完整命令（本来就打印在终端）。
- 与 L4 事件日志分离：审计是**安全视角的旁路**，不参与消息投影。

## 5. 验收标准

- `/plan on` 后让模型改文件 → 返回 `[MODE DENIED]`，文件未被修改。
- `run_shell("echo x > ~/.zshrc")` → 触发审批提示（当前直接通过）。
- `run_shell("ls; rm -rf /tmp/foo")` → 按 `rm` 的档位判定，不因 `ls` 在前而放行。
- `~/.seekcli/audit.jsonl` 能还原一次会话里所有工具调用与审批结果。

## 6. 对应路线

阶段二十四（边界一致性，P1）。
