use serde::{Deserialize, Serialize};
use std::collections::HashMap;
use std::path::Path;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TokenModel {
    pub name: String,
    pub provider: String,
    /// Cost per 1,000 input tokens in microcents (1/1,000,000 cent)
    pub input_price_per_1k: u64,
    /// Cost per 1,000 output tokens in microcents
    pub output_price_per_1k: u64,
    pub max_context: u64,
}

impl TokenModel {
    pub fn cost_for_tokens(&self, input_tokens: u64, output_tokens: u64) -> u64 {
        let input_cost = (input_tokens.saturating_add(999)) / 1000 * self.input_price_per_1k;
        let output_cost = (output_tokens.saturating_add(999)) / 1000 * self.output_price_per_1k;
        input_cost + output_cost
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
struct ModelRegistryData {
    models: Vec<TokenModel>,
}

#[derive(Debug, Clone)]
pub struct ModelRegistry {
    models: HashMap<String, TokenModel>,
}

impl Default for ModelRegistry {
    fn default() -> Self {
        Self::builtin()
    }
}

impl ModelRegistry {
    /// Built-in defaults shipped with the binary.
    pub fn builtin() -> Self {
        let mut models = HashMap::new();

        crate::insert_model!(models, "gpt-4", "openai", 30_000, 60_000, 8192);
        crate::insert_model!(models, "gpt-4-32k", "openai", 60_000, 120_000, 32768);
        crate::insert_model!(models, "gpt-4-turbo", "openai", 10_000, 30_000, 128000);
        crate::insert_model!(models, "gpt-4o", "openai", 5_000, 15_000, 128000);
        crate::insert_model!(models, "gpt-4o-mini", "openai", 150, 600, 128000);
        crate::insert_model!(models, "gpt-3.5-turbo", "openai", 500, 1500, 16385);
        crate::insert_model!(models, "claude-3-opus", "anthropic", 15_000, 75_000, 200000);
        crate::insert_model!(models, "claude-3-sonnet", "anthropic", 3_000, 15_000, 200000);
        crate::insert_model!(models, "claude-3-haiku", "anthropic", 250, 1250, 200000);
        crate::insert_model!(models, "llama-3-8b", "meta", 200, 200, 8192);
        crate::insert_model!(models, "llama-3-70b", "meta", 650, 650, 8192);
        crate::insert_model!(models, "mixtral-8x7b", "mistral", 400, 400, 32768);

        ModelRegistry { models }
    }

    /// Load model pricing from a JSON file. Falls back to built-in
    /// defaults for any model names not present in the file, so a
    /// partial config works as an overlay.
    pub fn from_json_file<P: AsRef<Path>>(path: P) -> Result<Self, String> {
        let data = std::fs::read_to_string(path.as_ref())
            .map_err(|e| format!("Failed to read models file: {}", e))?;
        Self::from_json_str(&data)
    }

    /// Parse model pricing from a JSON string. Merges with built-in
    /// defaults — file entries override, everything else stays.
    pub fn from_json_str(json: &str) -> Result<Self, String> {
        let data: ModelRegistryData = serde_json::from_str(json)
            .map_err(|e| format!("Failed to parse models JSON: {}", e))?;

        let mut registry = Self::builtin();
        for model in data.models {
            registry.models.insert(model.name.clone(), model);
        }
        Ok(registry)
    }

    pub fn get(&self, name: &str) -> Option<&TokenModel> {
        self.models.get(name)
    }

    pub fn list(&self) -> Vec<&TokenModel> {
        let mut all: Vec<&TokenModel> = self.models.values().collect();
        all.sort_by_key(|m| m.name.clone());
        all
    }

    pub fn cost_for(&self, model_name: &str, input_tokens: u64, output_tokens: u64) -> Option<u64> {
        self.models
            .get(model_name)
            .map(|m| m.cost_for_tokens(input_tokens, output_tokens))
    }
}

#[macro_export]
macro_rules! insert_model {
    ($map:expr, $name:expr, $provider:expr, $input:expr, $output:expr, $ctx:expr) => {
        $map.insert(
            $name.to_string(),
            TokenModel {
                name: $name.to_string(),
                provider: $provider.to_string(),
                input_price_per_1k: $input,
                output_price_per_1k: $output,
                max_context: $ctx,
            },
        );
    };
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_gpt4o_cost() {
        let registry = ModelRegistry::builtin();
        let cost = registry
            .cost_for("gpt-4o", 1000, 500)
            .expect("model not found");
        // (1000+999)/1000*5_000 + (500+999)/1000*15_000 = 1*5_000 + 1*15_000 = 20_000
        assert_eq!(cost, 20_000);
    }

    #[test]
    fn test_unknown_model() {
        let registry = ModelRegistry::builtin();
        assert!(registry.cost_for("nonexistent", 100, 100).is_none());
    }

    #[test]
    fn test_from_json_override() {
        let json = r#"{
            "models": [
                {"name": "gpt-4o", "provider": "openai", "input_price_per_1k": 9999, "output_price_per_1k": 8888, "max_context": 128000}
            ]
        }"#;
        let registry = ModelRegistry::from_json_str(json).unwrap();
        assert_eq!(registry.cost_for("gpt-4o", 1000, 1000), Some(9999 + 8888));
    }

    #[test]
    fn test_from_json_partial_keeps_builtin() {
        let json = r#"{"models": []}"#;
        let registry = ModelRegistry::from_json_str(json).unwrap();
        assert!(registry.get("gpt-4").is_some());
        assert_eq!(registry.list().len(), 12);
    }

    #[test]
    fn test_cost_rounds_up() {
        let registry = ModelRegistry::builtin();
        // 1 token rounds up to 1k for gpt-4: (1+999)/1000*30_000 + (1+999)/1000*60_000 = 90_000
        let cost = registry
            .cost_for("gpt-4", 1, 1)
            .expect("model not found");
        assert_eq!(cost, 90_000);
    }

    #[test]
    fn test_list_models_sorted() {
        let registry = ModelRegistry::builtin();
        let list = registry.list();
        assert!(list.len() >= 3);
        for pair in list.windows(2) {
            assert!(pair[0].name <= pair[1].name);
        }
    }
}
