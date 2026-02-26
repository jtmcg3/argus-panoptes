//! Research agent - web search, document analysis, and knowledge building.
//!
//! This agent performs research tasks by:
//! 1. Searching existing memory for relevant knowledge
//! 2. Performing web searches for new information
//! 3. Fetching and analyzing web content
//! 4. Synthesizing findings into coherent summaries
//! 5. Storing key findings in memory for future reference

use crate::traits::{Agent, AgentCapability, AgentConfig};
use async_trait::async_trait;
use panoptes_common::{AgentMessage, PanoptesError, Result, Task};
use panoptes_llm::{ChatMessage, LlmClient, LlmRequest, Role};
use panoptes_memory::{Memory, MemoryConfig, MemoryRetriever, MemoryStore, MemoryType};
use reqwest::Client;
use scraper::{Html, Selector};
use serde::{Deserialize, Serialize};
use std::sync::Arc;
use std::time::Duration;
use tokio::sync::Semaphore;
use tracing::{debug, info, warn};
use url::Url;

const RESEARCH_SYSTEM_PROMPT: &str = r#"You are a research assistant specialized in gathering and synthesizing information. Your role is to:

1. Understand research queries and identify key topics
2. Search for relevant, authoritative sources
3. Synthesize information into clear summaries
4. Cite sources properly
5. Identify gaps in knowledge and suggest follow-up research

Always prioritize accuracy over speed.
Distinguish between facts and opinions.
Note the recency and reliability of sources.
"#;

/// Configuration for the research agent.
#[derive(Debug, Clone)]
pub struct ResearchConfig {
    /// Maximum number of search results to process
    pub max_search_results: usize,
    /// Maximum content length to fetch per page (bytes)
    pub max_content_length: usize,
    /// Request timeout
    pub request_timeout: Duration,
    /// User agent for web requests
    pub user_agent: String,
    /// Whether to store findings in memory
    pub persist_findings: bool,
    /// Minimum importance score for storing findings
    pub min_importance_for_storage: f32,
    /// SearXNG instance URL for web search (e.g. "http://100.85.147.105:8888")
    /// Falls back to DuckDuckGo instant answers API if not set.
    pub search_url: Option<String>,
    /// Maximum concurrent research tasks (queued via semaphore)
    pub max_concurrent: usize,
}

impl Default for ResearchConfig {
    fn default() -> Self {
        Self {
            max_search_results: 5,
            max_content_length: 50_000,
            request_timeout: Duration::from_secs(30),
            user_agent: "Argus-Panoptes/0.1 (Research Agent)".into(),
            persist_findings: true,
            min_importance_for_storage: 0.6,
            search_url: None,
            max_concurrent: 3,
        }
    }
}

/// A search result from web search.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SearchResult {
    pub title: String,
    pub url: String,
    pub snippet: String,
}

/// Fetched and parsed web content.
#[derive(Debug, Clone)]
pub struct WebContent {
    pub url: String,
    pub title: String,
    pub text: String,
    pub fetch_time: std::time::Instant,
}

/// Research agent for web search and knowledge gathering.
pub struct ResearchAgent {
    config: AgentConfig,
    research_config: ResearchConfig,
    semaphore: Arc<Semaphore>,
    http_client: Client,
    /// Memory store for persisting and retrieving knowledge
    memory_store: Option<Arc<MemoryStore>>,
    /// Memory retriever for building context
    memory_retriever: Option<Arc<MemoryRetriever>>,
    /// Optional LLM client for enhanced output
    llm_client: Option<Arc<dyn LlmClient>>,
}

impl ResearchAgent {
    /// Create a new ResearchAgent with the given configuration.
    pub fn new(config: AgentConfig, research_config: ResearchConfig) -> Self {
        let http_client = Client::builder()
            .timeout(research_config.request_timeout)
            .user_agent(&research_config.user_agent)
            .build()
            .expect("Failed to create HTTP client");

        let semaphore = Arc::new(Semaphore::new(research_config.max_concurrent));

        Self {
            config,
            research_config,
            semaphore,
            http_client,
            memory_store: None,
            memory_retriever: None,
            llm_client: None,
        }
    }

    /// Create a ResearchAgent with default configuration.
    pub fn with_default_config() -> Self {
        Self::new(
            AgentConfig {
                id: "research".into(),
                name: "Research Agent".into(),
                mcp_servers: vec!["web-search".into()],
                ..Default::default()
            },
            ResearchConfig::default(),
        )
    }

    /// Attach a memory store for knowledge persistence.
    pub fn with_memory(mut self, store: Arc<MemoryStore>) -> Self {
        let retriever = Arc::new(MemoryRetriever::new(Arc::clone(&store), 4096));
        self.memory_store = Some(store);
        self.memory_retriever = Some(retriever);
        self
    }

    /// Set the SearXNG URL for web search.
    pub fn with_search_url(mut self, url: String) -> Self {
        self.research_config.search_url = Some(url);
        self
    }

    /// Attach an LLM client for enhanced output.
    pub fn with_llm(mut self, client: Arc<dyn LlmClient>) -> Self {
        self.llm_client = Some(client);
        self
    }

    /// Enhance template output using the LLM, with graceful degradation.
    async fn enhance_with_llm(&self, template_output: &str, task: &Task) -> String {
        if let Some(ref llm) = self.llm_client {
            let system_prompt = self.system_prompt().to_string();
            let request = LlmRequest {
                system_prompt: Some(system_prompt),
                messages: vec![ChatMessage {
                    role: Role::User,
                    content: format!(
                        "Task: {}\n\nBased on these search results, synthesize a comprehensive summary:\n{}",
                        task.description, template_output
                    ),
                }],
                temperature: Some(0.7),
                max_tokens: Some(4096),
            };

            match llm.complete(request).await {
                Ok(response) if !response.content.is_empty() => return response.content,
                Ok(_) => warn!(agent = %self.id(), "LLM returned empty response, using template"),
                Err(e) => {
                    warn!(agent = %self.id(), error = %e, "LLM failed, falling back to template")
                }
            }
        }
        template_output.to_string()
    }

    /// Initialize memory from config (alternative to with_memory).
    pub async fn init_memory(&mut self, config: MemoryConfig) -> anyhow::Result<()> {
        let store = Arc::new(MemoryStore::new(config).await?);
        let retriever = Arc::new(MemoryRetriever::new(Arc::clone(&store), 4096));
        self.memory_store = Some(store);
        self.memory_retriever = Some(retriever);
        Ok(())
    }

    /// Search memory for existing knowledge on a topic.
    async fn search_memory(&self, query: &str) -> Vec<Memory> {
        if let Some(ref store) = self.memory_store {
            match store
                .search(query, Some(MemoryType::Semantic), Some(5))
                .await
            {
                Ok(memories) => {
                    info!(
                        agent = %self.id(),
                        query = %query,
                        found = memories.len(),
                        "Found existing knowledge in memory"
                    );
                    memories
                }
                Err(e) => {
                    warn!(agent = %self.id(), error = %e, "Failed to search memory");
                    Vec::new()
                }
            }
        } else {
            Vec::new()
        }
    }

    /// Build context from memory for the research task.
    async fn build_memory_context(&self, query: &str) -> String {
        if let Some(ref retriever) = self.memory_retriever {
            match retriever.build_context(query, Some(self.id())).await {
                Ok(context) if !context.is_empty() => context,
                Ok(_) => String::new(),
                Err(e) => {
                    warn!(agent = %self.id(), error = %e, "Failed to build memory context");
                    String::new()
                }
            }
        } else {
            String::new()
        }
    }

    /// Store a research finding in memory.
    async fn store_finding(&self, content: &str, source: &str, importance: f32) {
        if !self.research_config.persist_findings {
            return;
        }

        if importance < self.research_config.min_importance_for_storage {
            debug!(
                agent = %self.id(),
                importance = importance,
                min = self.research_config.min_importance_for_storage,
                "Finding below importance threshold, not storing"
            );
            return;
        }

        if let Some(ref store) = self.memory_store {
            let memory = Memory::semantic(content, source)
                .with_agent(self.id())
                .with_importance(importance)
                .with_tags(vec!["research".into(), "web".into()]);

            if let Err(e) = store.add(memory, true).await {
                warn!(agent = %self.id(), error = %e, "Failed to store finding in memory");
            } else {
                info!(agent = %self.id(), source = %source, "Stored research finding in memory");
            }
        }
    }

    /// Perform a web search. Uses SearXNG if configured, falls back to DuckDuckGo.
    async fn web_search(&self, query: &str) -> Result<Vec<SearchResult>> {
        info!(agent = %self.id(), query = %query, "Performing web search");

        if let Some(ref search_url) = self.research_config.search_url {
            self.searxng_search(search_url, query).await
        } else {
            self.duckduckgo_search(query).await
        }
    }

    /// Search via SearXNG JSON API.
    async fn searxng_search(&self, base_url: &str, query: &str) -> Result<Vec<SearchResult>> {
        // SearXNG searches work best with concise queries, not full task descriptions.
        // Trim to the first ~200 chars to avoid URL length issues and improve relevance.
        let trimmed_query: String = query.chars().take(200).collect();
        let url = format!(
            "{}/search?q={}&format=json&categories=general",
            base_url.trim_end_matches('/'),
            urlencoding::encode(&trimmed_query)
        );

        let response = self
            .http_client
            .get(&url)
            .send()
            .await
            .map_err(|e| PanoptesError::Agent(format!("SearXNG request failed: {}", e)))?;

        if !response.status().is_success() {
            warn!(
                agent = %self.id(),
                status = %response.status(),
                "SearXNG returned error, falling back to DuckDuckGo"
            );
            return self.duckduckgo_search(query).await;
        }

        let data: serde_json::Value = response
            .json()
            .await
            .map_err(|e| PanoptesError::Agent(format!("Failed to parse SearXNG response: {}", e)))?;

        let mut results = Vec::new();

        if let Some(items) = data["results"].as_array() {
            for item in items.iter().take(self.research_config.max_search_results) {
                let title = item["title"].as_str().unwrap_or("").to_string();
                let url = item["url"].as_str().unwrap_or("").to_string();
                let snippet = item["content"].as_str().unwrap_or("").to_string();

                if !title.is_empty() && !url.is_empty() {
                    results.push(SearchResult {
                        title,
                        url,
                        snippet,
                    });
                }
            }
        }

        info!(
            agent = %self.id(),
            results = results.len(),
            "SearXNG search completed"
        );

        Ok(results)
    }

    /// Fallback search via DuckDuckGo instant answers API.
    async fn duckduckgo_search(&self, query: &str) -> Result<Vec<SearchResult>> {
        let url = format!(
            "https://api.duckduckgo.com/?q={}&format=json&no_html=1&skip_disambig=1",
            urlencoding::encode(query)
        );

        let response = self
            .http_client
            .get(url)
            .send()
            .await
            .map_err(|e| PanoptesError::Agent(format!("Search request failed: {}", e)))?;

        if !response.status().is_success() {
            return Err(PanoptesError::Agent(format!(
                "Search returned status {}",
                response.status()
            )));
        }

        let data: serde_json::Value = response
            .json()
            .await
            .map_err(|e| PanoptesError::Agent(format!("Failed to parse search response: {}", e)))?;

        let mut results = Vec::new();

        if let Some(abstract_text) = data["AbstractText"].as_str()
            && !abstract_text.is_empty()
        {
            results.push(SearchResult {
                title: data["Heading"].as_str().unwrap_or("").to_string(),
                url: data["AbstractURL"].as_str().unwrap_or("").to_string(),
                snippet: abstract_text.to_string(),
            });
        }

        if let Some(topics) = data["RelatedTopics"].as_array() {
            for topic in topics.iter().take(self.research_config.max_search_results) {
                if let Some(text) = topic["Text"].as_str() {
                    results.push(SearchResult {
                        title: topic["Text"]
                            .as_str()
                            .unwrap_or("")
                            .chars()
                            .take(100)
                            .collect(),
                        url: topic["FirstURL"].as_str().unwrap_or("").to_string(),
                        snippet: text.to_string(),
                    });
                }
            }
        }

        if results.is_empty() {
            debug!(agent = %self.id(), "No DuckDuckGo instant answer results");
        }

        Ok(results)
    }

    /// Fetch and parse content from a URL.
    async fn fetch_content(&self, url_str: &str) -> Result<WebContent> {
        debug!(agent = %self.id(), url = %url_str, "Fetching web content");

        let url =
            Url::parse(url_str).map_err(|e| PanoptesError::Agent(format!("Invalid URL: {}", e)))?;

        let response = self
            .http_client
            .get(url.as_str())
            .send()
            .await
            .map_err(|e| PanoptesError::Agent(format!("Fetch failed: {}", e)))?;

        if !response.status().is_success() {
            return Err(PanoptesError::Agent(format!(
                "Fetch returned status {}",
                response.status()
            )));
        }

        let content_type = response
            .headers()
            .get("content-type")
            .and_then(|v| v.to_str().ok())
            .unwrap_or("");

        // Only process HTML content
        if !content_type.contains("text/html") {
            return Err(PanoptesError::Agent(format!(
                "Unsupported content type: {}",
                content_type
            )));
        }

        let html = response
            .text()
            .await
            .map_err(|e| PanoptesError::Agent(format!("Failed to read response: {}", e)))?;

        // Limit content length
        let html = if html.len() > self.research_config.max_content_length {
            match html
                .char_indices()
                .nth(self.research_config.max_content_length)
            {
                Some((idx, _)) => html[..idx].to_string(),
                None => html,
            }
        } else {
            html
        };

        // Parse HTML and extract text
        let document = Html::parse_document(&html);

        // Extract title
        let title_selector = Selector::parse("title").unwrap();
        let title = document
            .select(&title_selector)
            .next()
            .map(|el| el.text().collect::<String>())
            .unwrap_or_else(|| url_str.to_string());

        // Extract main content (try common content selectors)
        let content_selectors = [
            "article",
            "main",
            ".content",
            "#content",
            ".post-content",
            ".entry-content",
            "body",
        ];

        let mut text = String::new();
        for selector_str in &content_selectors {
            if let Ok(selector) = Selector::parse(selector_str) {
                for element in document.select(&selector) {
                    // Skip script and style tags
                    let element_text: String = element
                        .text()
                        .filter(|t| !t.trim().is_empty())
                        .collect::<Vec<_>>()
                        .join(" ");

                    if !element_text.is_empty() {
                        text = element_text;
                        break;
                    }
                }
                if !text.is_empty() {
                    break;
                }
            }
        }

        // Clean up whitespace
        let text = text.split_whitespace().collect::<Vec<_>>().join(" ");

        // Truncate if too long
        let text = if text.len() > 5000 {
            match text.char_indices().nth(5000) {
                Some((idx, _)) => format!("{}...", &text[..idx]),
                None => text,
            }
        } else {
            text
        };

        Ok(WebContent {
            url: url_str.to_string(),
            title,
            text,
            fetch_time: std::time::Instant::now(),
        })
    }

    /// Synthesize research findings into a summary.
    fn synthesize_findings(
        &self,
        query: &str,
        memory_context: &str,
        search_results: &[SearchResult],
        fetched_content: &[WebContent],
    ) -> String {
        let mut summary = String::new();

        // Add query
        summary.push_str(&format!("## Research Query\n\n{}\n\n", query));

        // Add memory context if available
        if !memory_context.is_empty() {
            summary.push_str("## Prior Knowledge (from Memory)\n\n");
            summary.push_str(memory_context);
            summary.push_str("\n\n");
        }

        // Add search results
        if !search_results.is_empty() {
            summary.push_str("## Search Results\n\n");
            for (i, result) in search_results.iter().enumerate() {
                summary.push_str(&format!(
                    "### {}. {}\n**Source:** {}\n\n{}\n\n",
                    i + 1,
                    result.title,
                    result.url,
                    result.snippet
                ));
            }
        }

        // Add fetched content
        if !fetched_content.is_empty() {
            summary.push_str("## Detailed Content\n\n");
            for content in fetched_content {
                summary.push_str(&format!(
                    "### {}\n**URL:** {}\n\n{}\n\n",
                    content.title, content.url, content.text
                ));
            }
        }

        // Add synthesis section
        summary.push_str("## Key Findings\n\n");
        if search_results.is_empty() && fetched_content.is_empty() && memory_context.is_empty() {
            summary.push_str("No information found for this query. Consider:\n");
            summary.push_str("- Rephrasing the query\n");
            summary.push_str("- Breaking it into smaller, more specific questions\n");
            summary.push_str("- Checking spelling and terminology\n");
        } else {
            summary.push_str("Based on the above sources:\n\n");
            // Basic synthesis - count sources
            let source_count = search_results.len() + fetched_content.len();
            let has_memory = !memory_context.is_empty();
            summary.push_str(&format!(
                "- Found {} web source(s){}\n",
                source_count,
                if has_memory {
                    " plus prior knowledge from memory"
                } else {
                    ""
                }
            ));
        }

        summary
    }

    /// Execute a research task.
    async fn execute_research(&self, task: &Task) -> Result<AgentMessage> {
        let query = &task.description;

        info!(agent = %self.id(), query = %query, "Starting research task");

        // Step 1: Search memory for existing knowledge
        let memory_context = self.build_memory_context(query).await;
        let existing_memories = self.search_memory(query).await;

        // Step 2: Perform web search
        let search_results = match self.web_search(query).await {
            Ok(results) => results,
            Err(e) => {
                warn!(agent = %self.id(), error = %e, "Web search failed");
                Vec::new()
            }
        };

        // Step 3: Fetch detailed content from top results
        let mut fetched_content = Vec::new();
        for result in search_results.iter().take(3) {
            if result.url.is_empty() {
                continue;
            }
            match self.fetch_content(&result.url).await {
                Ok(content) => {
                    fetched_content.push(content);
                }
                Err(e) => {
                    debug!(
                        agent = %self.id(),
                        url = %result.url,
                        error = %e,
                        "Failed to fetch content"
                    );
                }
            }
        }

        // Step 4: Synthesize findings
        let summary =
            self.synthesize_findings(query, &memory_context, &search_results, &fetched_content);

        // Step 4b: Enhance with LLM if available
        let summary = self.enhance_with_llm(&summary, task).await;

        // Step 5: Store key findings in memory
        // Store each search result snippet as a semantic memory
        for result in &search_results {
            if !result.snippet.is_empty() && result.snippet.len() > 50 {
                let importance = if result.snippet.len() > 200 { 0.8 } else { 0.6 };
                self.store_finding(&result.snippet, &result.url, importance)
                    .await;
            }
        }

        // Store a summary memory for this query
        if !search_results.is_empty() || !fetched_content.is_empty() {
            let summary_memory = format!(
                "Research on '{}': Found {} results. Key sources: {}",
                query,
                search_results.len() + fetched_content.len(),
                search_results
                    .iter()
                    .take(3)
                    .map(|r| r.title.clone())
                    .collect::<Vec<_>>()
                    .join(", ")
            );
            self.store_finding(&summary_memory, "research-agent", 0.7)
                .await;
        }

        info!(
            agent = %self.id(),
            task_id = %task.id,
            search_results = search_results.len(),
            fetched_pages = fetched_content.len(),
            existing_memories = existing_memories.len(),
            "Research task completed"
        );

        let mut message = AgentMessage::from_agent(self.id(), summary);
        message.task_id = Some(task.id.clone());
        Ok(message)
    }
}

#[async_trait]
impl Agent for ResearchAgent {
    fn id(&self) -> &str {
        &self.config.id
    }

    fn name(&self) -> &str {
        &self.config.name
    }

    fn capabilities(&self) -> &[AgentCapability] {
        &[
            AgentCapability::WebSearch,
            AgentCapability::DocumentAnalysis,
            AgentCapability::MemoryAccess,
        ]
    }

    async fn process_task(&self, task: &Task) -> Result<AgentMessage> {
        info!(
            agent = %self.id(),
            task_id = %task.id,
            available_permits = self.semaphore.available_permits(),
            "Processing research task"
        );

        let _permit = self
            .semaphore
            .acquire()
            .await
            .map_err(|_| PanoptesError::Agent("Research semaphore closed".into()))?;

        self.execute_research(task).await
    }

    async fn handle_message(&self, message: &AgentMessage) -> Result<AgentMessage> {
        let task = Task::new(&message.content);
        self.process_task(&task).await
    }

    fn system_prompt(&self) -> &str {
        self.config
            .system_prompt
            .as_deref()
            .unwrap_or(RESEARCH_SYSTEM_PROMPT)
    }

    fn is_available(&self) -> bool {
        self.semaphore.available_permits() > 0
    }
}

/// URL encoding helper
mod urlencoding {
    pub fn encode(input: &str) -> String {
        input
            .chars()
            .map(|c| match c {
                'A'..='Z' | 'a'..='z' | '0'..='9' | '-' | '_' | '.' | '~' => c.to_string(),
                ' ' => "+".to_string(),
                _ => {
                    let mut buf = [0u8; 4];
                    let encoded = c.encode_utf8(&mut buf);
                    encoded
                        .bytes()
                        .map(|b| format!("%{:02X}", b))
                        .collect::<String>()
                }
            })
            .collect()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_research_agent_creation() {
        let agent = ResearchAgent::with_default_config();
        assert_eq!(agent.id(), "research");
        assert_eq!(agent.name(), "Research Agent");
        assert!(agent.is_available());
    }

    #[test]
    fn test_capabilities() {
        let agent = ResearchAgent::with_default_config();
        let caps = agent.capabilities();

        assert!(caps.contains(&AgentCapability::WebSearch));
        assert!(caps.contains(&AgentCapability::DocumentAnalysis));
        assert!(caps.contains(&AgentCapability::MemoryAccess));
    }

    #[test]
    fn test_url_encoding() {
        assert_eq!(urlencoding::encode("hello world"), "hello+world");
        assert_eq!(urlencoding::encode("rust async"), "rust+async");
        assert_eq!(urlencoding::encode("a&b=c"), "a%26b%3Dc");
    }

    #[test]
    fn test_synthesis_empty() {
        let agent = ResearchAgent::with_default_config();
        let summary = agent.synthesize_findings("test query", "", &[], &[]);

        assert!(summary.contains("test query"));
        assert!(summary.contains("No information found"));
    }

    #[test]
    fn test_synthesis_with_results() {
        let agent = ResearchAgent::with_default_config();
        let results = vec![SearchResult {
            title: "Test Result".into(),
            url: "https://example.com".into(),
            snippet: "This is a test snippet".into(),
        }];

        let summary = agent.synthesize_findings("test query", "", &results, &[]);

        assert!(summary.contains("Test Result"));
        assert!(summary.contains("test snippet"));
        assert!(summary.contains("Found 1 web source"));
    }

    #[test]
    fn test_synthesis_with_memory() {
        let agent = ResearchAgent::with_default_config();
        let memory_context = "Previously researched: Rust is a systems programming language.";

        let summary = agent.synthesize_findings("rust async", memory_context, &[], &[]);

        assert!(summary.contains("Prior Knowledge"));
        assert!(summary.contains("systems programming language"));
    }

    #[test]
    fn test_research_config_defaults() {
        let config = ResearchConfig::default();

        assert_eq!(config.max_search_results, 5);
        assert!(config.persist_findings);
        assert!(config.min_importance_for_storage > 0.0);
    }
}
