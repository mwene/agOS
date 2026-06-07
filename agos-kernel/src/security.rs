use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct AuditEntry {
    pub action: Value,
    pub result: String,
    pub reason: String,
    pub timestamp: String,
}

#[derive(Debug, Clone, serde::Serialize, serde::Deserialize)]
pub struct Constraint {
    pub field: String,
    pub operator: String,
    pub value: Value,
    pub rule: String,
}

pub struct SMTSolver {
    constraints: std::sync::RwLock<Vec<Constraint>>,
}

impl Default for SMTSolver {
    fn default() -> Self {
        Self::new()
    }
}

impl SMTSolver {
    pub fn new() -> Self {
        SMTSolver {
            constraints: std::sync::RwLock::new(Vec::new()),
        }
    }

    pub fn add_constraint(&self, constraint: Constraint) {
        let mut c = self.constraints.write().unwrap();
        c.push(constraint);
    }

    pub fn check(&self, claim: &Value) -> Result<(), String> {
        let constraints = self.constraints.read().unwrap();
        for c in constraints.iter() {
            if !evaluate_constraint(c, claim) {
                return Err(format!("Constraint violated: {}", c.rule));
            }
        }
        Ok(())
    }

    pub fn get_constraints(&self) -> Vec<Constraint> {
        self.constraints.read().unwrap().clone()
    }
}

fn evaluate_constraint(c: &Constraint, claim: &Value) -> bool {
    let claim_value = claim.get(&c.field);
    let Some(cv) = claim_value else { return false };

    match c.operator.as_str() {
        "==" => {
            if let (Some(a), Some(b)) = (cv.as_str(), c.value.as_str()) {
                a == b
            } else if let (Some(a), Some(b)) = (cv.as_f64(), c.value.as_f64()) {
                (a - b).abs() < f64::EPSILON
            } else {
                cv == &c.value
            }
        }
        "!=" => {
            if let (Some(a), Some(b)) = (cv.as_str(), c.value.as_str()) {
                a != b
            } else {
                cv != &c.value
            }
        }
        "<=" => {
            if let (Some(a), Some(b)) = (cv.as_f64(), c.value.as_f64()) {
                a <= b
            } else {
                false
            }
        }
        ">=" => {
            if let (Some(a), Some(b)) = (cv.as_f64(), c.value.as_f64()) {
                a >= b
            } else {
                false
            }
        }
        "in" => {
            if let (Some(val), Some(arr)) = (cv.as_str(), c.value.as_array()) {
                arr.iter()
                    .filter_map(|v| v.as_str())
                    .any(|v| v == val)
            } else {
                false
            }
        }
        "not_in" => {
            if let (Some(val), Some(arr)) = (cv.as_str(), c.value.as_array()) {
                !arr.iter()
                    .filter_map(|v| v.as_str())
                    .any(|v| v == val)
            } else {
                true
            }
        }
        _ => false,
    }
}

pub struct SecurityEngine {
    smt_solver: SMTSolver,
    audit_log: std::sync::RwLock<Vec<AuditEntry>>,
    #[cfg(feature = "z3-solver")]
    z3_solver: crate::z3_solver::Z3PolicySolver,
}

impl Default for SecurityEngine {
    fn default() -> Self {
        Self::new()
    }
}

impl SecurityEngine {
    pub fn new() -> Self {
        #[allow(unused_mut)]
        let mut engine = SecurityEngine {
            smt_solver: SMTSolver::new(),
            audit_log: std::sync::RwLock::new(Vec::new()),
            #[cfg(feature = "z3-solver")]
            z3_solver: crate::z3_solver::Z3PolicySolver::new(),
        };
        engine.init_default_policies();
        engine
    }

    fn init_default_policies(&self) {
        self.smt_solver.add_constraint(Constraint {
            field: "action".to_string(),
            operator: "not_in".to_string(),
            value: serde_json::json!(["network:write", "filesystem:write", "sandbox:escape"]),
            rule: "blocked_actions".to_string(),
        });
        self.smt_solver.add_constraint(Constraint {
            field: "capability".to_string(),
            operator: "!=".to_string(),
            value: serde_json::json!("admin:escalate"),
            rule: "no_escalation".to_string(),
        });
        self.smt_solver.add_constraint(Constraint {
            field: "amount".to_string(),
            operator: "<=".to_string(),
            value: serde_json::json!(0),
            rule: "no_financial_transfer".to_string(),
        });
    }

    pub fn ring0_verify(
        &self,
        action_vector: &Value,
        permissions: &[String],
    ) -> Result<(), String> {
        let action_str = action_vector
            .get("action")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let capability = action_vector
            .get("capability")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let target = action_vector
            .get("target")
            .and_then(|v| v.as_str())
            .unwrap_or("");
        let amount = action_vector
            .get("amount")
            .and_then(|v| v.as_f64())
            .unwrap_or(0.0);

        let claim = serde_json::json!({
            "action": action_str,
            "capability": capability,
            "target": target,
            "amount": amount,
        });

        if let Err(msg) = self.smt_solver.check(&claim) {
            self.audit_log.write().unwrap().push(AuditEntry {
                action: action_vector.clone(),
                result: "DENIED".to_string(),
                reason: msg.clone(),
                timestamp: Utc::now().to_rfc3339(),
            });
            return Err(msg);
        }

        #[cfg(feature = "z3-solver")]
        {
            let constraints = self.smt_solver.get_constraints();
            if let Err(msg) = self.z3_solver.verify(
                action_str,
                capability,
                target,
                amount,
                &constraints,
            ) {
                self.audit_log.write().unwrap().push(AuditEntry {
                    action: action_vector.clone(),
                    result: "DENIED".to_string(),
                    reason: msg.clone(),
                    timestamp: Utc::now().to_rfc3339(),
                });
                return Err(msg);
            }
        }

        if !permissions.is_empty() {
            let allowed = permissions.iter().any(|perm| match_permission(action_str, perm));
            if !allowed {
                self.audit_log.write().unwrap().push(AuditEntry {
                    action: action_vector.clone(),
                    result: "DENIED".to_string(),
                    reason: "Missing required permission for this action".to_string(),
                    timestamp: Utc::now().to_rfc3339(),
                });
                return Err("Missing required permission for this action".to_string());
            }
        }

        self.audit_log.write().unwrap().push(AuditEntry {
            action: action_vector.clone(),
            result: "ALLOWED".to_string(),
            reason: "verified".to_string(),
            timestamp: Utc::now().to_rfc3339(),
        });
        Ok(())
    }

    pub fn get_audit_log(&self, limit: usize) -> Vec<AuditEntry> {
        let log = self.audit_log.read().unwrap();
        log.iter().rev().take(limit).cloned().rev().collect()
    }
}

fn match_permission(action: &str, pattern: &str) -> bool {
    if pattern == "*" || pattern == "*:*" {
        return true;
    }
    let parts_a: Vec<&str> = action.split(':').collect();
    let parts_p: Vec<&str> = pattern.split(':').collect();
    for i in 0..std::cmp::max(parts_a.len(), parts_p.len()) {
        let p = parts_p.get(i).copied().unwrap_or("");
        let a = parts_a.get(i).copied().unwrap_or("");
        if p == "*" {
            continue;
        }
        if p != a {
            return false;
        }
    }
    true
}
