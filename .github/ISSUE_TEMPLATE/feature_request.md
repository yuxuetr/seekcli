---
name: 功能建议
about: 希望 SeekCLI 增加某项能力
labels: enhancement
---

## 想解决的问题

<!-- 描述你卡在哪，而不是你想要的实现。 -->

## 是否已被明确排除？

请先看一眼 `docs/architecture/design-principles.md` §2 与各层文档的「明确不做」。
不少能力是**刻意**不做的（沙箱、持久 PTY、workflow 编排、跨会话记忆、Web UI……），
它们各自写了理由。如果你认为某条理由不成立，直接说哪一条、为什么——那比新开一个提议更有用。

## 能不能用 MCP 解决？

SeekCLI 的扩展路径是 MCP：外部能力通过 `[[mcp]]` 接入，不需要改 Rust。
如果你的需求能表达成一个 MCP server，那多半不需要动核心。
