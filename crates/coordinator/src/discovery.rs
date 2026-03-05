//! Agent discovery via A2A Agent Cards.
//!
//! The `AgentRegistry` fetches Agent Cards from configured URLs on startup
//! and caches them. The coordinator uses this to route requests to the
//! correct agent based on skills.

use panoptes_a2a::AgentCard;
use panoptes_common::{PanoptesError, Result};
use std::collections::HashMap;
use tracing::{info, warn};

/// Registry of discovered A2A agents.
#[derive(Debug, Clone)]
pub struct AgentRegistry {
    /// Agent Cards keyed by agent name.
    agents: HashMap<String, RegisteredAgent>,
}

/// An agent discovered from its Agent Card.
#[derive(Debug, Clone)]
pub struct RegisteredAgent {
    pub card: AgentCard,
    pub base_url: String,
}

impl AgentRegistry {
    /// Create a new empty registry.
    pub fn new() -> Self {
        Self {
            agents: HashMap::new(),
        }
    }

    /// Discover agents by fetching Agent Cards from the given URLs.
    ///
    /// Each URL should be the agent's base URL (e.g. `http://localhost:9001`).
    /// The Agent Card is fetched from `{url}/.well-known/agent.json`.
    pub async fn discover(&mut self, agent_urls: &[String]) -> Result<()> {
        let client = reqwest::Client::builder()
            .timeout(std::time::Duration::from_secs(5))
            .build()
            .map_err(|e| PanoptesError::Config(format!("Failed to create HTTP client: {}", e)))?;

        for url in agent_urls {
            let card_url = format!("{}/.well-known/agent.json", url.trim_end_matches('/'));
            match client.get(&card_url).send().await {
                Ok(resp) if resp.status().is_success() => match resp.json::<AgentCard>().await {
                    Ok(card) => {
                        info!(
                            agent = %card.name,
                            url = %url,
                            skills = card.skills.len(),
                            "Discovered agent"
                        );
                        self.agents.insert(
                            card.name.clone(),
                            RegisteredAgent {
                                card,
                                base_url: url.trim_end_matches('/').to_string(),
                            },
                        );
                    }
                    Err(e) => {
                        warn!(url = %card_url, error = %e, "Failed to parse Agent Card");
                    }
                },
                Ok(resp) => {
                    warn!(url = %card_url, status = %resp.status(), "Agent Card request failed");
                }
                Err(e) => {
                    warn!(url = %card_url, error = %e, "Failed to reach agent");
                }
            }
        }

        info!(agents = self.agents.len(), "Agent discovery complete");

        Ok(())
    }

    /// Look up an agent by name.
    pub fn get(&self, name: &str) -> Option<&RegisteredAgent> {
        self.agents.get(name)
    }

    /// Find an agent that has a skill matching the given tag.
    pub fn find_by_skill_tag(&self, tag: &str) -> Option<&RegisteredAgent> {
        self.agents.values().find(|a| {
            a.card
                .skills
                .iter()
                .any(|s| s.tags.iter().any(|t| t == tag))
        })
    }

    /// Get all registered agents.
    pub fn all(&self) -> impl Iterator<Item = &RegisteredAgent> {
        self.agents.values()
    }

    /// Number of registered agents.
    pub fn len(&self) -> usize {
        self.agents.len()
    }

    /// Whether the registry is empty.
    pub fn is_empty(&self) -> bool {
        self.agents.is_empty()
    }

    /// Manually register an agent (for testing or static config).
    pub fn register(&mut self, base_url: String, card: AgentCard) {
        self.agents
            .insert(card.name.clone(), RegisteredAgent { card, base_url });
    }
}

impl Default for AgentRegistry {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use panoptes_a2a::{AgentCapabilities, AgentSkill};

    fn test_card(name: &str, tags: &[&str]) -> AgentCard {
        AgentCard {
            name: name.into(),
            description: format!("{} agent", name),
            url: Some(format!("http://localhost:9001/")),
            version: "0.1.0".into(),
            capabilities: AgentCapabilities {
                streaming: true,
                push_notifications: false,
            },
            skills: vec![AgentSkill {
                id: "test".into(),
                name: "Test".into(),
                description: "Test skill".into(),
                tags: tags.iter().map(|t| t.to_string()).collect(),
                examples: vec![],
            }],
            default_input_modes: vec!["text/plain".into()],
            default_output_modes: vec!["text/plain".into()],
        }
    }

    #[test]
    fn test_registry_register_and_lookup() {
        let mut registry = AgentRegistry::new();
        registry.register(
            "http://localhost:9001".into(),
            test_card("panoptes-research", &["research", "search"]),
        );

        assert_eq!(registry.len(), 1);
        assert!(registry.get("panoptes-research").is_some());
        assert!(registry.get("panoptes-writing").is_none());
    }

    #[test]
    fn test_registry_find_by_skill_tag() {
        let mut registry = AgentRegistry::new();
        registry.register(
            "http://localhost:9001".into(),
            test_card("panoptes-research", &["research", "search"]),
        );
        registry.register(
            "http://localhost:9002".into(),
            test_card("panoptes-writing", &["writing", "content"]),
        );

        let found = registry.find_by_skill_tag("research");
        assert!(found.is_some());
        assert_eq!(found.unwrap().card.name, "panoptes-research");

        let found = registry.find_by_skill_tag("writing");
        assert!(found.is_some());
        assert_eq!(found.unwrap().card.name, "panoptes-writing");

        assert!(registry.find_by_skill_tag("nonexistent").is_none());
    }
}
