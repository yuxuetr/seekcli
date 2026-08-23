---
name: Bug 报告
about: 模型行为异常、工具出错、或 SeekCLI 本身崩溃
labels: bug
---

## 现象

<!-- 实际发生了什么。如果模型「声称完成但没做」，请特别说明。 -->

## 复现

```bash
seekcli -p "..."
```

## 期望

## 环境

- SeekCLI 版本：`seekcli --version`
- 系统：
- provider / 模型：`~/.seekcli/config.toml` 的 `[brain]`（**不要贴 key**）

## 决策树（强烈建议）

用 `SEEKCLI_TRACE=1` 复现一次，附上 `~/.seekcli/traces/<id>.json`。
留意 `verdict` 字段：`answered` 且 `tool_calls: 0` 说明模型只是在说话、并没有动手。

> trace 含你的提示词内容，发之前请先过一眼。
