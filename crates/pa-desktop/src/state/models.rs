//! Model catalog entries, provider metadata, reasoning levels, and search filtering.

use std::collections::HashSet;

use serde_json::Value;

/// One selectable model from `get_model_catalog`.
#[derive(Debug, Clone, PartialEq)]
pub struct CatalogModel {
    pub provider: String,
    pub id: String,
    pub name: String,
    pub reasoning: bool,
    pub supports_images: bool,
    pub context_window: u64,
    pub max_tokens: u64,
    pub configured: bool,
    pub supported_levels: Vec<String>,
}

impl CatalogModel {
    #[must_use]
    pub fn selector(&self) -> String {
        format!("{}/{}", self.provider, self.id)
    }

    #[must_use]
    pub fn provider_display(&self) -> &str {
        provider_display_name(&self.provider)
    }

    #[must_use]
    pub fn matches_query(&self, query: &str) -> bool {
        let q = query.trim().to_ascii_lowercase();
        if q.is_empty() {
            return true;
        }
        let haystack = format!(
            "{} {} {} {}",
            self.provider,
            self.provider_display(),
            self.id,
            self.name
        )
        .to_ascii_lowercase();
        q.split_whitespace().all(|term| haystack.contains(term))
    }
}

/// Standard Prime Agent thinking effort levels.
pub const THINKING_LEVELS: [&str; 6] = ["off", "minimal", "low", "medium", "high", "xhigh"];

#[must_use]
pub fn provider_display_name(provider: &str) -> &str {
    match provider {
        "prime-inference" => "Prime Inference",
        "openai-codex" => "OpenAI Codex",
        "anthropic" => "Anthropic",
        "google" | "google-gemini-cli" | "google-vertex" => "Google Gemini",
        "openai" => "OpenAI",
        "amazon-bedrock" | "bedrock" => "AWS Bedrock",
        "mistral" => "Mistral",
        "azure-openai-responses" | "azure" => "Azure OpenAI",
        "openrouter" => "OpenRouter",
        "xai" => "xAI",
        other => other,
    }
}

#[must_use]
pub fn provider_sort_rank(provider: &str) -> u8 {
    match provider {
        "anthropic" => 0,
        "openai-codex" => 1,
        "prime-inference" => 2,
        "google" => 3,
        "openai" => 4,
        _ => 10,
    }
}

/// Decode `get_model_catalog` response `data` into sorted [`CatalogModel`] rows.
#[must_use]
pub fn parse_model_catalog(data: &Value) -> (Vec<CatalogModel>, HashSet<String>) {
    let configured: HashSet<String> = data
        .get("configuredProviders")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(Value::as_str)
                .map(str::to_string)
                .collect()
        })
        .unwrap_or_default();

    let mut models: Vec<CatalogModel> = data
        .get("models")
        .and_then(Value::as_array)
        .map(|arr| {
            arr.iter()
                .filter_map(|item| parse_one_model(item, &configured))
                .collect()
        })
        .unwrap_or_default();

    models.sort_by(|a, b| {
        b.configured
            .cmp(&a.configured)
            .then_with(|| provider_sort_rank(&a.provider).cmp(&provider_sort_rank(&b.provider)))
            .then_with(|| a.provider.cmp(&b.provider))
            .then_with(|| {
                a.name
                    .to_ascii_lowercase()
                    .cmp(&b.name.to_ascii_lowercase())
            })
    });

    (models, configured)
}

fn parse_one_model(item: &Value, configured: &HashSet<String>) -> Option<CatalogModel> {
    let obj = item.as_object()?;
    let provider = obj.get("provider").and_then(Value::as_str)?.to_string();
    let id = obj.get("id").and_then(Value::as_str)?.to_string();
    if provider.is_empty() || id.is_empty() {
        return None;
    }
    let name = obj
        .get("name")
        .and_then(Value::as_str)
        .filter(|s| !s.is_empty())
        .unwrap_or(&id)
        .to_string();
    let reasoning = obj
        .get("reasoning")
        .and_then(Value::as_bool)
        .unwrap_or(false);
    let supports_images = obj
        .get("input")
        .and_then(Value::as_array)
        .is_some_and(|inputs| inputs.iter().any(|v| v.as_str() == Some("image")));
    let context_window = obj
        .get("contextWindow")
        .and_then(Value::as_u64)
        .unwrap_or(128_000);
    let max_tokens = obj
        .get("maxTokens")
        .and_then(Value::as_u64)
        .unwrap_or(8_192);

    let mut supported_levels: Vec<String> = Vec::new();
    if let Some(level_map) = obj.get("thinkingLevelMap").and_then(Value::as_object) {
        for level in THINKING_LEVELS {
            if level_map.contains_key(level) {
                supported_levels.push(level.to_string());
            }
        }
    }
    if supported_levels.is_empty() && reasoning {
        supported_levels = THINKING_LEVELS.iter().map(|s| (*s).to_string()).collect();
    }

    Some(CatalogModel {
        configured: configured.contains(&provider),
        provider,
        id,
        name,
        reasoning,
        supports_images,
        context_window,
        max_tokens,
        supported_levels,
    })
}
