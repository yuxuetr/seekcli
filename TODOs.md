# SeekCLI 进化路线图 (TODOs)

本项目通过 **DeepSeek V4 (推理中枢)** + **GLM-4.6v / Qwen / StepFun (多模态感官)** 的混合架构，打造极客专用的增强型 CLI。

---

## ✅ 阶段一：API 协议重构 (兼容多模态)
- [x] 重构 `Message` 结构，支持多模态 ContentPart。
- [x] 多 Provider 适配器抽象 (DeepSeek, Zhipu, MinerU 等)。

## ✅ 阶段二：视觉理解管线 (Vision Pipeline)
- [x] 剪贴板位图捕获及后台解析。
- [x] 视觉预处理并注入到推理上下文中。

## ✅ 阶段三：文件解析与 `@` 语法 (File Sense)
- [x] `@` 语法解析器实现。
- [x] 集成 MinerU 进行复杂文档 (PDF/Docx) 解析。

## ✅ 阶段四：联网搜索插件 (Global Tools)
- [x] 接入 GLM Web Search 与 Tavily AI Search。

---

## 🚀 阶段五：Harness Agent 核心引擎构建
*目标：将 CLI 从“问答机器人”升级为具备自治、反思和纠错能力的自主智能体。*

- [x] **5.1 构建本地工具分发器 (Tool Dispatcher)**
    - [x] 创建 `src/tools` 模块，规范化工具执行接口。
    - [x] 实现基础系统工具：`read_file`, `write_file`, `list_dir`。
    - [x] 实现命令执行工具：`run_shell` (带 stdout/stderr 捕获机制)。
- [x] **5.2 重构 Agent 核心闭环 (The ReAct Loop)**
    - [x] 修改 `main.rs` 的 `chat` 方法，引入外层 `loop` 循环。
    - [x] 实现 `ToolCall` 的无缝拦截与本地执行。
    - [x] 将执行结果作为 `ToolResponse` 压入上下文并自动重新发起推理请求。
    - [x] 优化流式输出 UI，使工具调用与文本思考在终端展示时清晰分离。
- [x] **5.3 实现 Sub-Agent (子智能体委派) 机制**
    - [x] 在 `tools` 模块实现 `invoke_agent` 方法。
    - [x] 支持在执行该工具时，创建一个无上下文依赖的全新 `Session`，独立跑一轮完整的 Agent Loop。
    - [x] 将子任务的最终输出字符串作为 `ToolResponse` 汇报给主干循环，实现极简的上下文压缩。
- [x] **5.4 动态技能生成体系 (Dynamic Skill Generation)**
    - [x] 提供 `create_skill` 系统工具。
    - [x] 允许大模型将习得的最佳实践（System Prompt + 约束工具列表）自主序列化为 `.json` 并持久化到 `~/.seekcli/skills/` 目录。
    - [x] 测试 Agent 自主编写并加载新技能的完整流。

---

## 🛠️ 阶段六：UI/UX 与工程化增强
- [ ] **运行期终端安全审计**：对于危险命令（如 `rm -rf`）拦截并请求用户终端输入 `y/n` 确认。
- [ ] **多模态进度条**: 使用 `indicatif` 显示更平滑的“正在执行工具”、“后台子任务思考中...”状态。
- [ ] **代码解释器模式**：为复杂的数学或逻辑问题，自动生成并运行 Python 沙盒脚本获取结果。
