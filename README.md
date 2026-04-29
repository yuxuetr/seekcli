# SeekCLI

**SeekCLI** 是一个专为 DeepSeek 大模型深度定制的个人命令行效率工具。它不仅仅是一个简单的 API 调用客户端，而是围绕 DeepSeek V4 (1M Context) 的特性，专为开发者和重度 AI 用户设计的日常交互终端。

## 🌟 核心特性

- 🚀 **DeepSeek V4 原生支持**: 充分发挥 1M 超长上下文优势，无需频繁清理历史，支持深度长文本分析。
- 🧠 **多级思考模式**: 支持 `/thinking` 指令快速切换 [None/High/Max] 思考强度，适配不同难度的任务。
- 🎭 **智能技能系统 (Skills)**:
    - **自动路由**: 根据输入自动识别并切换预设技能（如：翻译专家、代码助手、文件处理）。
    - **系统提示词注入**: 针对不同场景自动优化 System Prompt。
- 🎨 **极致终端体验**:
    - **智能渲染**: 基于 `termimad` 的 Markdown 渲染，视觉清晰。
    - **语法高亮**: 代码块实时高亮（base16-ocean 主题），支持多种主流语言。
    - **快捷复制**: `/copy [idx]` 一键提取代码块到系统剪贴板。
- 💾 **会话管理**:
    - 自动保存历史记录至 `~/.seekcli/sessions/`。
    - 支持 `/history` 查看和 `/load` 加载历史会话。

## 🛠️ 安装与配置

### 环境要求
- Rust 2024 Edition (v1.85+)
- DeepSeek 或 阿里云 DashScope API Key

### 快速安装
```bash
git clone https://github.com/yuxuetr/seekcli.git
cd seekcli
cargo install --path .
```

### 配置 API Key
在环境变量中设置以下之一：
```bash
export DEEPSEEK_API_KEY="your_key"
# 或者使用 DashScope (Qwen/DeepSeek)
export DASHSCOPE_API_BASE="https://dashscope.aliyuncs.com/compatible-mode/v1"
export DASHSCOPE_API_KEY="your_key"
```

## ⌨️ 交互指令

进入交互模式后，可以使用以下指令提升效率：

| 指令 | 说明 | 示例 |
| :--- | :--- | :--- |
| `/model` | 切换模型 (flash/pro) | `/model pro` |
| `/thinking` | 调整思考强度 (n/h/m) | `/thinking h` |
| `/skill` | 管理/激活特定技能 | `/skill list`, `/skill translator` |
| `/copy` | 复制最后一次回复的代码块 | `/copy 1` |
| `/history` | 查看最近的会话记录 | `/history` |
| `/clear` | 重置当前会话上下文 | `/clear` |
| `/help` | 显示帮助菜单 | `/help` |
| `/quit` | 退出程序 | `/quit` |

## 📁 目录结构
- `~/.seekcli/sessions/`: 存放聊天的 JSON 记录。
- `~/.seekcli/skills/`: 存放自定义技能配置。

## 🤝 个人自用声明
本项目初衷是创建一个符合个人工作流的 DeepSeek 交互工具。如果你有好的想法或优化建议，欢迎提交 PR。

## 📄 开源协议
[MIT License](LICENSE)
