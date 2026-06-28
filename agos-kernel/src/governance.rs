use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Mutex;

use crate::sandbox::AAppInstance;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractRule {
    pub field: String,
    pub operator: String,
    pub value: Value,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ContractLimits {
    pub max_tokens_per_run: Option<u64>,
    pub max_subagents: Option<u32>,
    pub max_execution_seconds: Option<u64>,
    pub max_memory_pages: Option<u32>,
    pub allowed_permissions: Option<Vec<String>>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SuccessCriteria {
    pub min_goal_metric: Option<f64>,
    pub required_output_fields: Option<Vec<String>>,
    pub min_steps_completed: Option<u32>,
    pub no_error_states: Option<bool>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GovernanceContract {
    pub contract_id: String,
    pub name: String,
    pub description: String,
    pub rules: Vec<ContractRule>,
    pub limits: ContractLimits,
    pub success_criteria: SuccessCriteria,
    pub created_by: String,
}

impl GovernanceContract {
    pub fn new(name: &str, description: &str, created_by: &str) -> Self {
        GovernanceContract {
            contract_id: uuid::Uuid::new_v4().to_string(),
            name: name.to_string(),
            description: description.to_string(),
            rules: Vec::new(),
            limits: ContractLimits {
                max_tokens_per_run: None,
                max_subagents: None,
                max_execution_seconds: None,
                max_memory_pages: None,
                allowed_permissions: None,
            },
            success_criteria: SuccessCriteria {
                min_goal_metric: None,
                required_output_fields: None,
                min_steps_completed: None,
                no_error_states: None,
            },
            created_by: created_by.to_string(),
        }
    }

    pub fn with_rules(mut self, rules: Vec<ContractRule>) -> Self {
        self.rules = rules;
        self
    }

    pub fn with_limits(mut self, limits: ContractLimits) -> Self {
        self.limits = limits;
        self
    }

    pub fn with_success_criteria(mut self, criteria: SuccessCriteria) -> Self {
        self.success_criteria = criteria;
        self
    }

    pub fn check_compliance(&self, instance: &AAppInstance) -> ComplianceReport {
        let mut violations = Vec::new();

        for rule in &self.rules {
            match rule.operator.as_str() {
                "eq" | "==" => {
                    if !Self::match_value(&instance.manifest, &rule.field, &rule.value) {
                        violations.push(format!(
                            "Rule violated: {} must equal {:?}",
                            rule.field, rule.value
                        ));
                    }
                }
                "lt" | "<" => {
                    if let Some(val) = Self::extract_number(&instance.manifest, &rule.field) {
                        if let Some(ref limit) = rule.value.as_f64() {
                            if val >= *limit {
                                violations.push(format!(
                                    "Rule violated: {} ({}) must be less than {}",
                                    rule.field, val, limit
                                ));
                            }
                        }
                    }
                }
                "gt" | ">" => {
                    if let Some(val) = Self::extract_number(&instance.manifest, &rule.field) {
                        if let Some(ref limit) = rule.value.as_f64() {
                            if val <= *limit {
                                violations.push(format!(
                                    "Rule violated: {} ({}) must be greater than {}",
                                    rule.field, val, limit
                                ));
                            }
                        }
                    }
                }
                "contains" => {
                    if let Some(arr) = Self::extract_array(&instance.manifest, &rule.field) {
                        if !arr.iter().any(|v| v == &rule.value) {
                            violations.push(format!(
                                "Rule violated: {} must contain {:?}",
                                rule.field, rule.value
                            ));
                        }
                    }
                }
                _ => {
                    violations.push(format!("Unknown operator: {}", rule.operator));
                }
            }
        }

        if let Some(ref max_tokens) = self.limits.max_tokens_per_run {
            if let Some(actual) = Self::extract_number(&instance.manifest, "limits.max_tokens") {
                if actual as u64 > *max_tokens {
                    violations.push(format!(
                        "Limit exceeded: max_tokens {} > {}",
                        actual, max_tokens
                    ));
                }
            }
        }

        if let Some(ref max_sub) = self.limits.max_subagents {
            if let Some(actual) = Self::extract_number(&instance.manifest, "limits.max_subagents") {
                if actual as u32 > *max_sub {
                    violations.push(format!(
                        "Limit exceeded: max_subagents {} > {}",
                        actual, max_sub
                    ));
                }
            }
        }

        if let Some(ref allowed) = self.limits.allowed_permissions {
            if let Some(perms) = instance.manifest.get("permissions").and_then(|v| v.as_array()) {
                for p in perms {
                    if let Some(p_str) = p.as_str() {
                        if !allowed.contains(&p_str.to_string()) {
                            violations.push(format!(
                                "Permission '{}' is not in the allowed list",
                                p_str
                            ));
                        }
                    }
                }
            }
        }

        let compliant = violations.is_empty();
        ComplianceReport {
            contract_id: self.contract_id.clone(),
            contract_name: self.name.clone(),
            agent_name: instance.name.clone(),
            compliant,
            violations,
            timestamp: chrono::Utc::now().to_rfc3339(),
        }
    }

    fn match_value(manifest: &Value, field: &str, expected: &Value) -> bool {
        let parts: Vec<&str> = field.split('.').collect();
        let mut current = manifest;
        for part in &parts {
            match current.get(*part) {
                Some(v) => current = v,
                None => return false,
            }
        }
        current == expected
    }

    fn extract_number(manifest: &Value, field: &str) -> Option<f64> {
        let parts: Vec<&str> = field.split('.').collect();
        let mut current = manifest;
        for part in &parts {
            match current.get(*part) {
                Some(v) => current = v,
                None => return None,
            }
        }
        current.as_f64().or_else(|| current.as_u64().map(|n| n as f64))
    }

    fn extract_array<'a>(manifest: &'a Value, field: &str) -> Option<&'a Vec<Value>> {
        let parts: Vec<&str> = field.split('.').collect();
        let mut current = manifest;
        for part in &parts {
            match current.get(*part) {
                Some(v) => current = v,
                None => return None,
            }
        }
        current.as_array()
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComplianceReport {
    pub contract_id: String,
    pub contract_name: String,
    pub agent_name: String,
    pub compliant: bool,
    pub violations: Vec<String>,
    pub timestamp: String,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct GovernancePolicy {
    pub policy_id: String,
    pub name: String,
    pub description: String,
    pub contracts: Vec<String>,
    pub is_active: bool,
}

pub struct ContractEngine {
    contracts: Mutex<HashMap<String, GovernanceContract>>,
    policies: Mutex<HashMap<String, GovernancePolicy>>,
    audit_log: Mutex<Vec<ComplianceReport>>,
}

impl Default for ContractEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl ContractEngine {
    pub fn new() -> Self {
        ContractEngine {
            contracts: Mutex::new(HashMap::new()),
            policies: Mutex::new(HashMap::new()),
            audit_log: Mutex::new(Vec::new()),
        }
    }

    pub fn register_contract(&self, contract: GovernanceContract) -> Result<(), String> {
        let mut contracts = self.contracts.lock().unwrap();
        if contracts.contains_key(&contract.contract_id) {
            return Err(format!("Contract {} already exists", contract.contract_id));
        }
        contracts.insert(contract.contract_id.clone(), contract);
        Ok(())
    }

    pub fn get_contract(&self, contract_id: &str) -> Option<GovernanceContract> {
        self.contracts.lock().unwrap().get(contract_id).cloned()
    }

    pub fn list_contracts(&self) -> Vec<GovernanceContract> {
        let contracts = self.contracts.lock().unwrap();
        let mut list: Vec<GovernanceContract> = contracts.values().cloned().collect();
        list.sort_by(|a, b| a.name.cmp(&b.name));
        list
    }

    pub fn remove_contract(&self, contract_id: &str) -> bool {
        self.contracts.lock().unwrap().remove(contract_id).is_some()
    }

    pub fn check_agent_compliance(&self, instance: &AAppInstance) -> Vec<ComplianceReport> {
        let contracts = self.contracts.lock().unwrap();
        let mut reports = Vec::new();
        for contract in contracts.values() {
            let report = contract.check_compliance(instance);
            self.audit_log.lock().unwrap().push(report.clone());
            reports.push(report);
        }
        reports
    }

    pub fn get_audit_log(&self, limit: Option<usize>) -> Vec<ComplianceReport> {
        let log = self.audit_log.lock().unwrap();
        let limit = limit.unwrap_or(log.len());
        let start = log.len().saturating_sub(limit);
        log.iter().skip(start).take(limit).cloned().collect()
    }

    pub fn get_compliance_summary(&self) -> ComplianceSummary {
        let contracts = self.contracts.lock().unwrap();
        let audit = self.audit_log.lock().unwrap();

        let total_checks = audit.len();
        let compliant_checks = audit.iter().filter(|r| r.compliant).count();
        let violation_checks = total_checks - compliant_checks;

        let mut top_violations: HashMap<String, usize> = HashMap::new();
        for report in audit.iter() {
            for v in &report.violations {
                *top_violations.entry(v.clone()).or_insert(0) += 1;
            }
        }
        let mut violations: Vec<(String, usize)> = top_violations.into_iter().collect();
        violations.sort_by(|a, b| b.1.cmp(&a.1));
        violations.truncate(10);

        ComplianceSummary {
            total_contracts: contracts.len(),
            total_checks,
            compliant_checks,
            violation_checks,
            top_violations: violations,
        }
    }

    pub fn create_policy(&self, name: &str, description: &str, contract_ids: Vec<String>) -> GovernancePolicy {
        let mut policies = self.policies.lock().unwrap();
        let policy = GovernancePolicy {
            policy_id: uuid::Uuid::new_v4().to_string(),
            name: name.to_string(),
            description: description.to_string(),
            contracts: contract_ids,
            is_active: true,
        };
        policies.insert(policy.policy_id.clone(), policy.clone());
        policy
    }

    pub fn list_policies(&self) -> Vec<GovernancePolicy> {
        let policies = self.policies.lock().unwrap();
        let mut list: Vec<GovernancePolicy> = policies.values().cloned().collect();
        list.sort_by(|a, b| a.name.cmp(&b.name));
        list
    }
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ComplianceSummary {
    pub total_contracts: usize,
    pub total_checks: usize,
    pub compliant_checks: usize,
    pub violation_checks: usize,
    pub top_violations: Vec<(String, usize)>,
}

impl std::fmt::Display for ComplianceSummary {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(
            f,
            "Compliance Summary:\n  Contracts: {}\n  Checks: {}\n  Compliant: {}\n  Violations: {}\n  Top Violations: {:?}",
            self.total_contracts, self.total_checks, self.compliant_checks, self.violation_checks, self.top_violations
        )
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::collections::HashMap;
    use crate::sandbox::AAppInstance;

    fn test_instance() -> AAppInstance {
        AAppInstance {
            name: "test-agent".to_string(),
            path: "/tmp/test".to_string(),
            manifest: serde_json::json!({
                "name": "test-agent",
                "version": "1.0.0",
                "capabilities": ["test:run"],
                "permissions": ["umi:reason"],
                "limits": {
                    "max_tokens": 1000,
                    "max_subagents": 2
                },
                "priority": 1,
                "goal_metric": 0.85,
            }),
            logic_graphs: serde_json::json!({"initial_state": "start", "states": {}}),
            tools: HashMap::new(),
            workflow: None,
            process: None,
        }
    }

    #[test]
    fn test_contract_creation() {
        let contract = GovernanceContract::new("test-contract", "A test contract", "user-1");
        assert_eq!(contract.name, "test-contract");
        assert!(!contract.contract_id.is_empty());
    }

    #[test]
    fn test_contract_with_rules() {
        let contract = GovernanceContract::new("limits", "Resource limits", "admin")
            .with_rules(vec![
                ContractRule {
                    field: "priority".to_string(),
                    operator: "lt".to_string(),
                    value: Value::from(5),
                },
            ]);
        assert_eq!(contract.rules.len(), 1);
    }

    #[test]
    fn test_compliance_rule_passes() {
        let contract = GovernanceContract::new("strict", "Strict limits", "admin")
            .with_rules(vec![
                ContractRule {
                    field: "limits.max_tokens".to_string(),
                    operator: "lt".to_string(),
                    value: Value::from(2000),
                },
            ])
            .with_limits(ContractLimits {
                max_tokens_per_run: Some(2000),
                max_subagents: Some(5),
                max_execution_seconds: None,
                max_memory_pages: None,
                allowed_permissions: Some(vec!["umi:reason".to_string()]),
            });

        let instance = test_instance();
        let report = contract.check_compliance(&instance);

        assert!(report.compliant);
    }

    #[test]
    fn test_compliance_token_violation() {
        let contract = GovernanceContract::new("token-limit", "Token limit", "admin")
            .with_limits(ContractLimits {
                max_tokens_per_run: Some(500),
                max_subagents: None,
                max_execution_seconds: None,
                max_memory_pages: None,
                allowed_permissions: None,
            });

        let instance = test_instance();
        let report = contract.check_compliance(&instance);

        assert!(!report.compliant);
        assert!(report.violations.iter().any(|v| v.contains("max_tokens")));
    }

    #[test]
    fn test_compliance_permission_violation() {
        let contract = GovernanceContract::new("perm-limit", "Permission limits", "admin")
            .with_limits(ContractLimits {
                max_tokens_per_run: None,
                max_subagents: None,
                max_execution_seconds: None,
                max_memory_pages: None,
                allowed_permissions: Some(vec!["financial:read".to_string()]),
            });

        let instance = test_instance();
        let report = contract.check_compliance(&instance);

        assert!(!report.compliant);
        assert!(report.violations.iter().any(|v| v.contains("Permission")));
    }

    #[test]
    fn test_contract_engine() {
        let engine = ContractEngine::new();
        let contract = GovernanceContract::new("test", "desc", "admin");
        engine.register_contract(contract).unwrap();
        assert_eq!(engine.list_contracts().len(), 1);
    }

    #[test]
    fn test_audit_log() {
        let engine = ContractEngine::new();
        let contract = GovernanceContract::new("test", "desc", "admin")
            .with_limits(ContractLimits {
                max_tokens_per_run: Some(500),
                max_subagents: None,
                max_execution_seconds: None,
                max_memory_pages: None,
                allowed_permissions: None,
            });
        engine.register_contract(contract).unwrap();

        let instance = test_instance();
        engine.check_agent_compliance(&instance);

        let log = engine.get_audit_log(None);
        assert_eq!(log.len(), 1);
        assert!(!log[0].compliant);
    }

    #[test]
    fn test_summary() {
        let engine = ContractEngine::new();
        let contract = GovernanceContract::new("test", "desc", "admin")
            .with_limits(ContractLimits {
                max_tokens_per_run: Some(500),
                max_subagents: None,
                max_execution_seconds: None,
                max_memory_pages: None,
                allowed_permissions: None,
            });
        engine.register_contract(contract).unwrap();

        let instance = test_instance();
        engine.check_agent_compliance(&instance);

        let summary = engine.get_compliance_summary();
        assert_eq!(summary.total_contracts, 1);
        assert_eq!(summary.total_checks, 1);
        assert_eq!(summary.violation_checks, 1);
    }

    #[test]
    fn test_policy_creation() {
        let engine = ContractEngine::new();
        let contract = GovernanceContract::new("c1", "desc", "admin");
        let id = contract.contract_id.clone();
        engine.register_contract(contract).unwrap();

        let policy = engine.create_policy("prod-policy", "Prod deployment policy", vec![id]);
        assert!(!policy.contracts.is_empty());
        assert!(policy.is_active);

        let policies = engine.list_policies();
        assert_eq!(policies.len(), 1);
        assert_eq!(policies[0].name, "prod-policy");
    }
}
