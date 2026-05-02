# SeekCLI 进化路线图 (TODOs)

本项目通过 **DeepSeek V4 (推理中枢)** + **GLM-4.6v / Qwen (多模态感官)** 的混合架构，打造极客专用的增强型 CLI。

---

## 📅 阶段一：API 协议重构 (兼容多模态)
- [ ] **重构 `Message` 结构**:
    - 参考 GLM/Qwen 协议，将 `content` 从单一字符串改为 `Array<ContentPart>`。
    - 支持 `{"type": "text", "text": "..."}`。
    - 支持 `{"type": "image_url", "image_url": {"url": "..."}}` (含 Base64 编码支持)。
    - 支持 `{"type": "file_url", "file_url": {"url": "..."}}` (GLM 特色功能)。
- [ ] **多 Provider 适配器**:
    - 抽象 `ChatCompletion` 接口，支持 DeepSeek, DashScope (Qwen), Zhipu (GLM) 三个端点。
    - 引入 `thinking` 参数开关，适配 GLM-4.6v 的深度思考模式。

---

## 🎨 阶段二：视觉理解管线 (Vision Pipeline)
- [ ] **剪贴板位图捕获**:
    - 完善 `/copy` 对应的 `/paste` 指令。
    - 自动将剪贴板图片转换为临时 Base64。
- [ ] **视觉预处理 (Visual Sensor)**:
    - 选定 `glm-4.6v-flash` 或 `qwen3.6-plus` 作为视觉解析器。
    - **流程**: 用户贴图 -> 调用 Sensor 模型 -> 获取详细图像描述 (含 OCR) -> 将描述注入当前 Session。
- [ ] **DeepSeek 消费解析结果**:
    - 将 Sensor 返回的文本描述作为“视觉证据”传递给 DeepSeek V4 进行最终推理。

---

## 📄 阶段三：文件解析与 `@` 语法 (File Sense)
- [ ] **`@` 语法解析器**:
    - 在输入端检测 `@path/to/file`。
- [ ] **集成 MinerU (Magic-PDF) 传感器**:
    - 接入 `mineru.net` API，作为 PDF/Docx 的首选解析引擎。
    - 实现“提交任务 -> 轮询状态 -> 获取 Markdown”的异步流。
    - 将 MinerU 提取的高保真 Markdown 直接注入 DeepSeek V4 上下文。
- [ ] **混合路由逻辑**:
    - 纯文本 -> 直接读入。
    - 图片 -> GLM-4.6V 视觉解析。
    - PDF/复杂文档 -> MinerU 深度提取。
- [ ] **智能上下文管理**:
    - 自动判断文件大小，超大文件优先进行摘要或分段处理。

---

## 🔍 阶段四：联网搜索插件 (Global Tools)
- [ ] **集成 GLM Web Search**:
    - 将搜索作为全局 Tool 调用。
    - 允许 DeepSeek 在需要时触发联网查询，由 Rust 后端调用 GLM 搜索 API 并回传结果。

---

## ⚡ 阶段五：进阶技能系统与沙箱执行 (Execution)
- [ ] **本地工具映射**: 实现 `read_file`, `list_dir` 等基础 Rust 函数调用。
- [ ] **Python 代码解释器**: 支持模型生成 Python 脚本并在本地受限环境中运行（处理数据、绘图）。

---

## 🛠️ 阶段六：UI/UX 增强
- [ ] **多模态进度条**: 显示“📸 正在解析图片...”、“🔍 正在检索网络...”等状态。
- [ ] **渲染优化**: 支持在终端渲染图片链接的缩略占位符。

---

## 💡 选型参考
- **主思考引擎**: DeepSeek-V4-Pro (1M Context)
- **多模态感官**:
    - 视觉/文件：`glm-4.6v-flash` (支持 `thinking` 深度解析)
    - 备选视觉：`qwen3.6-plus` (阿里云 Bailian)
- **搜索服务**: GLM-4-Tools
