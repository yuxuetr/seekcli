# SeekCLI

**SeekCLI** 是一个专为 DeepSeek 大模型深度定制的个人命令行效率工具。它通过 **DeepSeek V4 (推理中枢)** + **多模态传感器 (感知引擎)** 的混合架构，将 AI 的思考能力与现实世界的数据获取能力完美结合。

## 🌟 核心特性

- 🚀 **DeepSeek V4 原生支持**: 充分发挥 1M 超长上下文优势，支持深度长文本分析。
- 👁️ **全能感官集成 (Multimodal Sensors)**:
    - **视觉分析**: 集成 VLM (Step-1V/GLM-4V)，一键解析剪贴板图片。
    - **文档解析**: 集成 MinerU (Magic-PDF)，支持 PDF/Docx/PPTX 高保真转 Markdown。
    - **网页抓取**: 集成 Jina Reader，自动提取无广告的正文内容。
    - **联网搜索**: 集成 GLM search_pro 和 Tavily AI，获取全网实时知识。
- 🧠 **对话即注入 (@ 语法)**:
    - 在聊天中通过 `@` 符号即时注入外部数据。例如：`翻译这段文字 @image` 或 `总结这个报告 @report.pdf`。
- 🛠️ **双模式运行**:
    - **交互模式**: 沉浸式多轮对话体验。
    - **单次模式**: 直接在终端运行，如 `seekcli search "Rust 新特性"`。
- 🎨 **极致终端体验**:
    - **智能渲染**: 基于 `termimad` 的 Markdown 渲染，视觉清晰。
    - **语法高亮**: 代码块实时高亮，支持 `/copy` 一键复制代码。

## 🛠️ 安装与配置

### 环境要求
- Rust 2024 Edition (v1.85+)
- 系统依赖: `unzip` (用于解析 MinerU 压缩包)

### 快速安装
```bash
git clone https://github.com/yuxuetr/seekcli.git
cd seekcli
cargo install --path .
```

### 配置环境变量
在 `.zshrc` 或 `.bashrc` 中配置以下 Key 以开启完整功能：
```bash
# 核心脑部 (必选)
export DEEPSEEK_API_KEY="your_key"

# 传感器集群 (可选)
export ZHIPU_API_KEY="your_key"    # 用于 /search 和 @search
export TAVILY_API_KEY="your_key"   # 用于 /tavily 和 @tavily
export JINA_API_KEY="your_key"     # 用于 /web 和 @url
export STEP_API_KEY="your_key"     # 用于 /image 和 @image
export MINERU_API_KEY="your_key"   # 用于 /file 和 @pdf
```

## ⌨️ 交互指令

### 1. 感知与工具 (不计入对话历史)
| 指令 | 说明 |
| :--- | :--- |
| `/image` | 解析剪贴板中的图片并预览内容 |
| `/file <path>` | 调用 MinerU 解析本地文档 (PDF/PPTX/Docx) |
| `/web <url>` | 解析指定网页的正文 |
| `/search <query>` | 使用智谱高阶引擎进行联网搜索 |
| `/tavily <query>` | 使用 Tavily 进行深度 AI 联网搜索 |

### 2. 对话集成 (@ 语法)
在聊天输入中加入以下标签，即可将解析结果注入给 DeepSeek：
- `@image` 或 `@img`: 注入剪贴板图片描述。
- `@https://...`: 注入指定网页内容。
- `@文件名.pdf`: 注入文档解析出的 Markdown。
- `@search` 或 `@tavily`: 注入联网搜索获取的信息证据。

### 3. 会话管理
| 指令 | 说明 |
| :--- | :--- |
| `/model [flash/pro]` | 切换 DeepSeek 模型 |
| `/thinking [n/h/m]` | 调整思考强度 (None/High/Max) |
| `/copy [index]` | 复制回复中的代码块 |
| `/history` | 查看历史会话列表 |
| `/clear` | 重置当前会话上下文 |

## 📁 目录结构
- `~/.seekcli/sessions/`: 存放聊天的 JSON 记录。
- `~/.seekcli/skills/`: 存放自定义技能配置。

## 🤝 贡献与反馈
本项目初衷是创建一个符合开发者工作流的 AI 交互终端。欢迎提交 Issue 或 PR。

## 📄 开源协议
[MIT License](LICENSE)
