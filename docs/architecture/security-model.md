# 安全边界声明

> SeekCLI 是**单用户本地 CLI**，不是生产服务。
> L3 的设计目标是「在工具结构允许的范围内提供常识性护栏」，而不是「完整 OS 隔离」。
> 实现细节与补全方案见 [L3-security.md](L3-security.md)。

## 1. 实际拦截清单

### 命令审批 —— `tools/approval.rs::classify`

三态 `Decision { Allow, Ask, Deny }`，优先级：
**user deny > 内置 deny > user allow > 内置 ask > allow**。

内置识别的危险模式（token 边界匹配）：

- `rm -rf` 触及 `/` | `~` | `$HOME` | 任何绝对路径
- `sudo` 提权
- `curl | sh` / `wget | bash` 远程脚本管道
- `dd of=/dev/...` 块设备写
- fork bomb `:(){...}`
- `chmod 777` / `chmod -R 777`
- `git push --force` / `git push -f`
- `mkfs*` 文件系统格式化

命中 `Ask` 时在 stderr 弹 `y/N`；用户拒绝返回 `[USER DENIED]` 前缀，
系统提示明确要求「见到 DENIED 不要重试同一调用」。
命中 `Deny`（catastrophic）直接拦截，不询问。

用户可在 `config.toml [security]` 用 allow / deny 子串列表扩展。

### 路径策略 —— `tools/path_security.rs::ensure_within_cwd`

词法归一化（不走 `canonicalize`，故可处理尚不存在的目标文件）+ `starts_with` 校验。
绝对路径或 `../..` 逃逸到 cwd 之外被拒，返回 `[PATH DENIED]` 前缀。

**当前覆盖**：`write_file`、`edit_file`。
**计划覆盖**：`run_shell` 的写意图路径（[L3 §4.1](L3-security.md#41-统一策略门l3-1--l3-2)）。

## 2. 明确不拦截的内容

### shell 重定向到 cwd 之外 —— *部分改变中*

`run_shell("date > /tmp/foo")` 当前会成功执行。

- **原有理由**：shell 写文件方式无穷多（`>` `>>` `tee` `cp` `mv` heredoc …），
  可靠拦截需要解析 shell AST，与「轻量 CLI」定位相悖。
- **修订**：完整 AST 解析仍不做，但 [L3 §4.1](L3-security.md#41-统一策略门l3-1--l3-2)
  将加入**轻量写意图 + 路径提取**，把工作区外写降级为 `Ask`。
  目标是「显著提高绕过成本」，**不是「不可绕过」**。

### `read_file` / `list_dir` 跨目录

模型可以读 `~/.zshrc` 等任意可读文件。
理由：`run_shell` 已有同等 exfiltration 能力（`cat ~/.ssh/id_rsa`），
单独限制 read 路径只损体验不增实际安全。

### 网络访问 / 子进程派生 / 数据外发

进程级别无任何限制。
理由：需要 OS 级 sandbox（chroot / Docker / firejail / seccomp / Landlock / Seatbelt），
与「用户本地 CLI」定位冲突。**如需该级别隔离，请在容器内运行 SeekCLI。**

## 3. 横向对比

| 项目 | 危险命令审批 | fs 路径检查 | 进程沙箱 |
| --- | --- | --- | --- |
| **SeekCLI** | ✅ token 匹配 | ✅ write / edit 限 cwd | ❌ 不做 |
| Claude Code | ✅ 用户允许列表 | ✅ write 限 workspace | ❌ 不做 |
| Hermes Agent | ✅ `approval.py` + `tool_guardrails.py` | ✅ `path_security.py` | ❌ 不做 |
| OpenAI Codex CLI | ✅ 类似 | ✅ 类似 | ⚠️ macOS sandbox-exec 可选 |
| **deepseek-harness** | ✅ permission presets | ✅ fs 观察策略 | ✅ bwrap / Landlock / Seatbelt 三后端 |

业界主流 CLI 方案与 SeekCLI 一致：**工具层做语义护栏，不在 CLI 层做完整 OS 沙箱**。
dsh 做到了沙箱，但它同时承担 Web / 远程 / 多租场景，威胁模型不同。

## 4. 用户责任声明

启动 SeekCLI 等同于**给一个智能模型授予终端访问权限**。用户应理解：

1. **审计 stdout**：`run_shell` 执行前会打印 `[Agent Executing] <command>`，应当注意阅读。
2. **不在敏感目录里运行**：避免在 `~/.ssh/`、含财务资料的目录下直接启动。
3. **重要数据先备份**：与所有自动化工具一样。
4. **如需更强隔离**：在 Docker 容器内运行，并只 mount 需要的目录。

## 5. 已知的边界不自洽（修复中）

以下两项**不是取舍，是缺陷**——声明的边界与实际执行的边界不一致，
用户会按声明去信任它。已列入阶段二十四：

| 项 | 现状 | 目标 |
| --- | --- | --- |
| Plan Mode | 只注入 prompt，不阻断写工具 | 模式门禁，写工具直接 `Deny` |
| shell 写路径 | 完全不检查 | 工作区外写意图降级为 `Ask` |

## 6. 未来增强方向（不在主线）

- OS sandbox 集成（macOS `sandbox-exec` / Linux Landlock / seccomp）
- `run_shell` 的严格只读模式（仅允许白名单命令集合）

审计日志（`~/.seekcli/audit.jsonl`）原属本节，
现已提升为阶段二十四的正式任务（[L3 §4.3](L3-security.md#43-审计日志l3-4)）。
