# 贡献指南

## 先读什么

改代码前请先读对应的设计文档——这个项目的大部分决定都写下了**为什么**，
以及**刻意不做什么**：

| 想改什么 | 先读 |
| --- | --- |
| 任意一层的设计意图 | `docs/architecture/L<n>-*.md` 的「目标设计」与「明确不做」 |
| 为什么要做这件事 | `docs/evaluation/`（缺口编号 L0-1 ~ L8-2） |
| 该不该做这件事 | `docs/architecture/design-principles.md` |
| 现在做到哪了 | `TODOs.md` |

每层文档都有「明确不做」小节。里面的项是权衡过的，要推翻请先改文档再动代码——
`scripts/check-gap-coverage.py` 会在 CI 里强制「评估 → 设计 → 路线」这条链不断。

## 开发环境

```bash
cargo build
cargo test --all
pre-commit install      # 强烈建议：钩子跑的就是 CI 跑的那套
```

需要 `DEEPSEEK_API_KEY` 才能真实跑起来；但**单测不需要**——
agent 主循环的测试走 `tests/fixtures/` 里录好的回放。

## 提交前

```bash
cargo fmt && cargo clippy --all-targets --all-features --tests -- -D warnings
cargo test --all
cargo deny check                       # 依赖公告审计，容易忘但 CI 会拦
python3 scripts/check-gap-coverage.py  # 溯源校验
```

pre-commit 会跑全套。**不要用 `--no-verify` 绕过**——这个仓库里已经出现过两个
长期静默失效的钩子（`cargo deny` 的参数、`black` 的版本），代价是问题多藏了很久。

## 代码风格

- **2 空格缩进**（覆盖 rustfmt 默认，见 `.rustfmt.toml`）
- **禁止 `unwrap()` / `expect()`**——测试、示例、构建脚本除外。
  遇到时先解决根因：这个 `Option` 为什么可能是 `None`？能否从类型设计上消除？
- 注释写**为什么**，不写做了什么。本仓库注释密度偏高且都在解释权衡，跟着这个风格写。

## 改动 LLM 交互时要格外小心

这一层的 bug 不会编译失败、不会 panic，只会让模型「宣称完成但什么都没做」。

- 改 `messages` 序列形状前，先想清楚是否破坏 assistant→user/tool→assistant 交替。
- 往系统提示加规则前，检查它在「无工具的规划轮」是否仍然成立。
- 用 `SEEKCLI_TRACE=1` 跑一遍，看决策树里该轮 `tool_calls` 是不是真的非零
  （`verdict` 字段会直接告诉你 acted / answered / empty）。
- 改动主循环后跑 `src/loop_tests.rs` 的回放测试；它们断言**副作用**而非模型说了什么。
- 新增轨迹：`SEEKCLI_RECORD=tests/fixtures/<name> seekcli -p "..."`。

## 提交规范

Conventional Commits，正文用中文：

```
<type>(<scope>): <一句话说明>

[TODOs.md 阶段<n> <n.m>]

<为什么这么改，以及权衡了什么>
```

- type：`feat` / `fix` / `docs` / `refactor` / `test` / `chore` / `perf` / `ci`
- **一个任务 = 一个逻辑提交**
- **不要添加 AI 署名 footer**

## 报告问题

`~/.seekcli/traces/` 里的决策树对诊断极有帮助（`SEEKCLI_TRACE=1` 开启）。
附上它比描述现象有用得多。注意 trace 含你的提示词内容——发之前先过一眼。
