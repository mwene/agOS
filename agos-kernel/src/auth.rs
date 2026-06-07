use std::collections::HashMap;

#[derive(Debug, Clone, Default)]
pub struct AuthConfig {
    pub api_key: Option<String>,
    pub agent_tokens: HashMap<String, String>,
}

impl AuthConfig {
    pub fn check_api_key(&self, header: Option<&str>) -> Result<(), String> {
        if let Some(ref key) = self.api_key {
            match header {
                Some(val) if val == key => Ok(()),
                _ => Err("Missing or invalid API key".into()),
            }
        } else {
            Ok(())
        }
    }

    pub fn check_agent_token(&self, agent_id: &str, token: &str) -> Result<(), String> {
        match self.agent_tokens.get(agent_id) {
            Some(expected) if expected == token => Ok(()),
            Some(_) => Err(format!("Invalid token for agent '{}'", agent_id)),
            None => {
                if token.is_empty() {
                    Ok(())
                } else {
                    Err(format!("Unknown agent '{}'", agent_id))
                }
            }
        }
    }
}
