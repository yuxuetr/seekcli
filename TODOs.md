# SeekCLI 进化路线图 (TODOs)

本项目通过 **DeepSeek V4 (推理中枢)** + **GLM-4.6v / Qwen / StepFun (多模态感官)** 的混合架构，打造极客专用的增强型 CLI。

---

## ✅ 阶段一：API 协议重构 (兼容多模态)
- [x] **重构 `Message` 结构**:
    - 支持 `ContentPart` 数组，兼容文本、图片、文件 URL。
- [x] **多 Provider 适配器**:
    - 抽象 `ApiClient` 接口，支持 DeepSeek, Zhipu (GLM), StepFun, MinerU, Jina 等端点。
    - 引入 `thinking` 参数开关。

---

## ✅ 阶段二：视觉理解管线 (Vision Pipeline)
- [x] **剪贴板位图捕获**:
    - 实现 `/image` 指令及后台 `osascript` 捕获逻辑。
- [x] **视觉预处理 (Visual Sensor)**:
    - 集成 `step-1v` 作为视觉解析器，获取详细图像描述并注入上下文。
- [x] **DeepSeek 消费解析结果**:
    - 将视觉描述作为“视觉证据”传递给 DeepSeek V4 进行最终推理。

---

## ✅ 阶段三：文件解析与 `@` 语法 (File Sense)
- [x] **`@` 语法解析器**:
    - 实现对 `@image`, `@file`, `@url` 的正则识别，支持带空格引号的路径。
- [x] **集成 MinerU (Magic-PDF) 传感器**:
    - 接入 `mineru.net` V4 API，支持 PDF/Docx 高保真还原。
- [x] **混合路由逻辑**:
    - 自动根据文件后缀和协议（http/https）选择对应的传感器。

---

## ✅ 阶段四：联网搜索插件 (Global Tools)
- [x] **集成 GLM Web Search**:
    - 对接智谱 `search_pro` 引擎。
- [x] **集成 Tavily AI Search**:
    - 对接 Tavily Advanced 搜索 API。
- [x] **独立与集成双模式**:
    - 支持 `/search` 独立预览与 `@search` 消息注入。

---

## 📅 阶段五：进阶技能系统与沙箱执行 (Execution)
- [ ] **本地工具映射**: 实现 `read_file`, `list_dir` 等基础 Rust 函数调用。
- [ ] **Python 代码解释器**: 支持模型生成 Python 脚本并在本地受限环境中运行。
- [ ] **上下文压缩**: 自动对长解析结果（如大 PDF）进行摘要，优化 Token 消耗。

---

## 🛠️ 阶段六：UI/UX 增强
- [ ] **多模态进度条**: 使用 `indicatif` 显示“📸 正在解析...”、“🔍 正在搜索...”等动态状态。
- [ ] **顶级 CLI 命令完善**: 进一步优化 `seekcli` 命令的参数传递和输出管道。

---

## 💡 选型参考
- **主思考引擎**: DeepSeek-V4-Pro (1M Context)
- **多模态感官**:
    - 视觉：`step-1.5v-mini` (高效) / `glm-4v`
    - 文件：`MinerU` (高保真)
    - 网页：`Jina Reader`
    - 搜索：`Tavily` / `GLM Search Pro`
