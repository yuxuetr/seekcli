use std::collections::VecDeque;
use std::pin::Pin;

use anyhow::{Context, Result};
use bytes::Bytes;
use futures_util::{Stream, StreamExt};
use reqwest::Client;
use serde::{Deserialize, Serialize};

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ContentPart {
  #[serde(rename = "type")]
  pub part_type: String,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub text: Option<String>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub image_url: Option<ImageUrl>,
  #[serde(skip_serializing_if = "Option::is_none")]
  pub file_url: Option<FileUrl>,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ImageUrl {
  pub url: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FileUrl {
  pub url: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(untagged)]
pub enum MessageContent {
  Text(String),
  Parts(Vec<ContentPart>),
}

#[derive(Debug, Serialize, Deserialize, Clone)]
#[serde(untagged)]
pub enum Message {
  Simple {
    role: String,
    content: MessageContent,
    #[serde(skip_serializing_if = "Option::is_none")]
    reasoning_content: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    tool_calls: Option<Vec<ToolCall>>,
  },
  ToolResponse {
    role: String,
    content: String,
    tool_call_id: String,
  },
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct ToolCall {
  pub id: String,
  #[serde(rename = "type")]
  pub tool_type: String,
  pub function: FunctionCall,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FunctionCall {
  pub name: String,
  pub arguments: String,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct Tool {
  #[serde(rename = "type")]
  pub tool_type: String,
  pub function: FunctionDefinition,
}

#[derive(Debug, Serialize, Deserialize, Clone)]
pub struct FunctionDefinition {
  pub name: String,
  pub description: String,
  pub parameters: serde_json::Value,
}

#[derive(Debug, Clone, Copy, PartialEq)]
pub enum Provider {
  DeepSeek,
  Zhipu,
  DashScope,
  MinerU,
  StepFun,
}

impl Provider {
  pub fn default_base_url(&self) -> &'static str {
    match self {
      Provider::DeepSeek => "https://api.deepseek.com/v1",
      Provider::Zhipu => "https://open.bigmodel.cn/api/paas/v4",
      Provider::DashScope => "https://dashscope.aliyuncs.com/compatible-mode/v1",
      Provider::MinerU => "https://mineru.net/api/v4",
      Provider::StepFun => "https://api.stepfun.com/v1",
    }
  }
}

#[derive(Debug, Deserialize)]
pub struct MineruResponse<T> {
  pub code: i32,
  pub data: Option<T>,
  pub msg: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct MineruBatchInfo {
  pub batch_id: String,
  pub file_urls: Vec<String>,
}

#[derive(Debug, Deserialize)]
pub struct MineruFileResult {
  #[allow(dead_code)]
  pub file_name: String,
  pub state: String,
  pub full_zip_url: Option<String>,
  pub err_msg: Option<String>,
}

#[derive(Debug, Deserialize)]
pub struct MineruBatchResult {
  #[allow(dead_code)]
  pub batch_id: String,
  pub extract_result: Vec<MineruFileResult>,
}

pub struct MineruResult {
  pub task_id: String,
  pub state: String,
  pub markdown_url: Option<String>,
  pub err_msg: Option<String>,
}

impl MineruResult {
  pub fn is_done(&self) -> bool {
    self.state == "done"
  }
}

pub struct ApiClient {
  client: Client,
  api_key: String,
  pub base_url: String,
  _provider: Provider,
  jina_api_key: Option<String>,
  tavily_api_key: Option<String>,
}

#[derive(Debug, Clone)]
pub enum StreamItem {
  Reasoning(String),
  Content(String),
  ToolCall(ToolCall),
  Finish(Option<String>),
}

struct StreamState {
  stream: Pin<Box<dyn Stream<Item = reqwest::Result<Bytes>> + Send>>,
  buffer: Vec<u8>,
  pending: VecDeque<Result<StreamItem>>,
  finished: bool,
}

impl Message {
  pub fn new_user_text(text: String) -> Self {
    Self::Simple {
      role: "user".to_string(),
      content: MessageContent::Text(text),
      reasoning_content: None,
      tool_calls: None,
    }
  }

  pub fn new_user_image(text: String, base64_image: String, mime_type: &str) -> Self {
    Self::Simple {
      role: "user".to_string(),
      content: MessageContent::Parts(vec![
        ContentPart {
          part_type: "text".to_string(),
          text: Some(text),
          image_url: None,
          file_url: None,
        },
        ContentPart {
          part_type: "image_url".to_string(),
          text: None,
          image_url: Some(ImageUrl {
            url: format!("data:{};base64,{}", mime_type, base64_image),
          }),
          file_url: None,
        },
      ]),
      reasoning_content: None,
      tool_calls: None,
    }
  }

  pub fn new_user_file(text: String, base64_file: String, mime_type: &str) -> Self {
    Self::Simple {
      role: "user".to_string(),
      content: MessageContent::Parts(vec![
        ContentPart {
          part_type: "file_url".to_string(),
          text: None,
          image_url: None,
          file_url: Some(FileUrl {
            url: format!("data:{};base64,{}", mime_type, base64_file),
          }),
        },
        ContentPart {
          part_type: "text".to_string(),
          text: Some(text),
          image_url: None,
          file_url: None,
        },
      ]),
      reasoning_content: None,
      tool_calls: None,
    }
  }
}

impl ApiClient {
  pub fn new(
    api_key: String,
    provider: Provider,
    jina_api_key: Option<String>,
    tavily_api_key: Option<String>,
  ) -> Self {
    let base_url =
      match provider {
        Provider::DeepSeek => std::env::var("DEEPSEEK_API_BASE")
          .unwrap_or_else(|_| provider.default_base_url().to_string()),
        Provider::Zhipu => std::env::var("ZHIPU_API_BASE")
          .unwrap_or_else(|_| provider.default_base_url().to_string()),
        Provider::DashScope => std::env::var("DASHSCOPE_API_BASE")
          .unwrap_or_else(|_| provider.default_base_url().to_string()),
        Provider::MinerU => std::env::var("MINERU_API_BASE")
          .unwrap_or_else(|_| provider.default_base_url().to_string()),
        Provider::StepFun => std::env::var("STEPFUN_API_BASE")
          .unwrap_or_else(|_| provider.default_base_url().to_string()),
      };

    let client = Client::builder()
      .no_proxy()
      .build()
      .unwrap_or_else(|_| Client::new());

    Self {
      client,
      api_key,
      base_url,
      _provider: provider,
      jina_api_key,
      tavily_api_key,
    }
  }

  pub async fn mineru_extract(&self, file_path: &std::path::Path) -> Result<String> {
    let file_name = file_path
      .file_name()
      .and_then(|s| s.to_str())
      .unwrap_or("file.pdf")
      .to_string();

    // 1. 获取上传 URL (V4 批量接口)
    let body = serde_json::json!({
        "files": [{
            "name": file_name,
        }],
        "is_ocr": true,
        "model_version": "vlm"
    });

    let resp = self
      .client
      .post(format!("{}/file-urls/batch", self.base_url))
      .header("Authorization", format!("Bearer {}", self.api_key))
      .json(&body)
      .send()
      .await?
      .error_for_status()?
      .json::<MineruResponse<MineruBatchInfo>>()
      .await?;

    if resp.code != 0 {
      anyhow::bail!(
        "MinerU Error ({}): {}",
        resp.code,
        resp.msg.unwrap_or_default()
      );
    }

    let data = resp
      .data
      .context("MinerU returned success but no data field")?;
    let batch_id = data.batch_id;
    let upload_url = data
      .file_urls
      .first()
      .context("No upload URL returned from MinerU")?;

    // 2. 使用 PUT 上传原始二进制文件 (V4 要求不设 Content-Type)
    let bytes = std::fs::read(file_path)?;
    self
      .client
      .put(upload_url)
      .body(bytes)
      .send()
      .await?
      .error_for_status()?;

    Ok(batch_id)
  }

  pub async fn mineru_get_result(&self, batch_id: &str) -> Result<MineruResult> {
    let resp = self
      .client
      .get(format!(
        "{}/extract-results/batch/{}",
        self.base_url, batch_id
      ))
      .header("Authorization", format!("Bearer {}", self.api_key))
      .send()
      .await?
      .error_for_status()?
      .json::<MineruResponse<MineruBatchResult>>()
      .await?;

    if resp.code != 0 {
      anyhow::bail!(
        "MinerU Result Error ({}): {}",
        resp.code,
        resp.msg.unwrap_or_default()
      );
    }

    let data = resp.data.context("MinerU result data missing")?;
    let file_res = data
      .extract_result
      .first()
      .context("No file result in batch")?;

    Ok(MineruResult {
      task_id: batch_id.to_string(),
      state: file_res.state.clone(),
      markdown_url: file_res.full_zip_url.clone(),
      err_msg: file_res.err_msg.clone(),
    })
  }

  pub async fn fetch_url_content(&self, url: &str) -> Result<String> {
    let resp = self.client.get(url).send().await?.error_for_status()?;

    if url.ends_with(".zip") {
      let bytes = resp.bytes().await?;
      let temp_dir = std::env::temp_dir();
      let zip_path = temp_dir.join(format!("{}.zip", uuid::Uuid::new_v4()));
      std::fs::write(&zip_path, bytes)?;

      // 1. 先列出所有文件，找到以 full.md 结尾的路径
      let list_output = std::process::Command::new("unzip")
        .arg("-Z")
        .arg("-1")
        .arg(&zip_path)
        .output()
        .context("Failed to execute unzip -Z command.")?;

      let file_list = String::from_utf8_lossy(&list_output.stdout);
      let target_file = file_list
        .lines()
        .map(|s| s.trim())
        .find(|s| s.ends_with("full.md"))
        .context("Could not find 'full.md' in MinerU result ZIP.")?;

      // 2. 提取找到的具体文件
      let output = std::process::Command::new("unzip")
        .arg("-p")
        .arg(&zip_path)
        .arg(target_file)
        .output()
        .context("Failed to execute unzip -p command.")?;

      // 清理临时文件
      let _ = std::fs::remove_file(&zip_path);

      if !output.status.success() {
        anyhow::bail!(
          "Failed to extract {} from ZIP: {}",
          target_file,
          String::from_utf8_lossy(&output.stderr)
        );
      }

      return Ok(String::from_utf8_lossy(&output.stdout).to_string());
    }

    Ok(resp.text().await?)
  }

  pub async fn fetch_web_markdown(&self, url: &str) -> Result<String> {
    let jina_url = format!("https://r.jina.ai/{}", url);
    let mut req = self
      .client
      .get(&jina_url)
      .header("X-With-Generated-Alt", "true");

    if let Some(ref key) = self.jina_api_key {
      req = req.header("Authorization", format!("Bearer {}", key));
    }

    let resp = req.send().await?.error_for_status()?;

    Ok(resp.text().await?)
  }

  pub async fn tavily_search(&self, query: &str) -> Result<String> {
    let key = self
      .tavily_api_key
      .as_ref()
      .context("TAVILY_API_KEY not set")?;
    let body = serde_json::json!({
        "query": query,
        "search_depth": "advanced",
        "include_answer": true,
        "max_results": 5
    });

    let resp = self
      .client
      .post("https://api.tavily.com/search")
      .header("Authorization", format!("Bearer {}", key))
      .json(&body)
      .send()
      .await?
      .error_for_status()?
      .json::<serde_json::Value>()
      .await?;

    let mut result = String::new();
    if let Some(answer) = resp["answer"].as_str() {
      result.push_str(&format!("Summary: {}\n\n", answer));
    }

    if let Some(results) = resp["results"].as_array() {
      for (i, res) in results.iter().enumerate() {
        result.push_str(&format!(
          "{}. [{}]({})\n{}\n\n",
          i + 1,
          res["title"].as_str().unwrap_or("No Title"),
          res["url"].as_str().unwrap_or("#"),
          res["content"].as_str().unwrap_or("")
        ));
      }
    }

    Ok(result)
  }

  pub async fn glm_web_search(&self, query: &str) -> Result<String> {
    let body = serde_json::json!({
        "search_query": query,
        "search_engine": "search_pro",
        "search_intent": false,
        "count": 10,
        "search_recency_filter": "noLimit",
        "content_size": "medium"
    });

    let resp = self
      .client
      .post("https://open.bigmodel.cn/api/paas/v4/web_search")
      .header("Authorization", format!("Bearer {}", self.api_key))
      .json(&body)
      .send()
      .await?
      .error_for_status()?
      .json::<serde_json::Value>()
      .await?;

    let mut results = String::new();
    if let Some(list) = resp["search_result"].as_array() {
      for (i, res) in list.iter().enumerate() {
        results.push_str(&format!(
          "{}. [{}]({})\n{}\n\n",
          i + 1,
          res["title"].as_str().unwrap_or("No Title"),
          res["link"].as_str().unwrap_or("#"),
          res["content"].as_str().unwrap_or("")
        ));
      }
    }

    if results.is_empty() {
      results.push_str("No search results found.");
    }

    Ok(results)
  }

  pub async fn call_api_with_params(
    &self,
    model: &str,
    messages: Vec<Message>,
    thinking_mode: &str,
    tools: Option<Vec<Tool>>,
  ) -> Result<Pin<Box<dyn Stream<Item = Result<StreamItem>> + Send>>> {
    let mut body = serde_json::json!({
      "model": model,
      "messages": messages,
      "stream": true,
    });

    if thinking_mode != "none" {
      body["thinking"] = serde_json::json!({
        "mode": thinking_mode,
      });
    }

    if let Some(t) = tools {
      body["tools"] = serde_json::json!(t);
    }

    let resp = self
      .client
      .post(format!("{}/chat/completions", self.base_url))
      .header("Authorization", format!("Bearer {}", self.api_key))
      .json(&body)
      .send()
      .await?;

    if !resp.status().is_success() {
      let status = resp.status();
      let err_body = resp.text().await?;
      anyhow::bail!("API Error {}: {}", status, err_body);
    }

    let stream = resp.bytes_stream();
    Ok(Box::pin(StreamState {
      stream: Box::pin(stream),
      buffer: Vec::new(),
      pending: VecDeque::new(),
      finished: false,
    }))
  }
}

impl Stream for StreamState {
  type Item = Result<StreamItem>;

  fn poll_next(
    mut self: Pin<&mut Self>,
    cx: &mut std::task::Context<'_>,
  ) -> std::task::Poll<Option<Self::Item>> {
    if let Some(item) = self.pending.pop_front() {
      return std::task::Poll::Ready(Some(item));
    }

    if self.finished {
      return std::task::Poll::Ready(None);
    }

    while let std::task::Poll::Ready(maybe_bytes) = self.stream.poll_next_unpin(cx) {
      match maybe_bytes {
        Some(Ok(bytes)) => {
          self.buffer.extend_from_slice(&bytes);
          let mut start = 0;
          while let Some(line_end) = self.buffer[start..].iter().position(|&b| b == b'\n') {
            let line_pos = start + line_end;
            let line = String::from_utf8_lossy(&self.buffer[start..line_pos]);
            start = line_pos + 1;

            if let Some(data) = line.strip_prefix("data: ") {
              let data = data.trim();
              if data == "[DONE]" {
                self.finished = true;
                break;
              }

              if let Ok(val) = serde_json::from_str::<serde_json::Value>(data)
                && let Some(choices) = val["choices"].as_array()
                && let Some(choice) = choices.first()
              {
                let delta = &choice["delta"];

                if let Some(reasoning) = delta["reasoning_content"].as_str()
                  && !reasoning.is_empty()
                {
                  self
                    .pending
                    .push_back(Ok(StreamItem::Reasoning(reasoning.to_string())));
                }

                if let Some(content) = delta["content"].as_str()
                  && !content.is_empty()
                {
                  self
                    .pending
                    .push_back(Ok(StreamItem::Content(content.to_string())));
                }

                if let Some(tool_calls) = delta["tool_calls"].as_array() {
                  for tc in tool_calls {
                    if let Ok(tool_call) = serde_json::from_value::<ToolCall>(tc.clone()) {
                      self.pending.push_back(Ok(StreamItem::ToolCall(tool_call)));
                    }
                  }
                }

                if let Some(finish_reason) = choice["finish_reason"].as_str() {
                  self
                    .pending
                    .push_back(Ok(StreamItem::Finish(Some(finish_reason.to_string()))));
                  self.finished = true;
                }
              }
            }
          }
          self.buffer.drain(..start);

          if let Some(item) = self.pending.pop_front() {
            return std::task::Poll::Ready(Some(item));
          }
          if self.finished {
            return std::task::Poll::Ready(None);
          }
        }
        Some(Err(e)) => return std::task::Poll::Ready(Some(Err(anyhow::Error::from(e)))),
        None => {
          self.finished = true;
          return std::task::Poll::Ready(None);
        }
      }
    }

    std::task::Poll::Pending
  }
}
