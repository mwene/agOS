use std::collections::HashMap;
use z3::ast::{Ast, Bool, Int};
use z3::{Config, Context, SatResult, Solver};

use crate::security::Constraint;

const ACTION: &str = "action";
const CAPABILITY: &str = "capability";
const TARGET: &str = "target";

pub struct Z3PolicySolver {
    ctx: Context,
}

unsafe impl Send for Z3PolicySolver {}
unsafe impl Sync for Z3PolicySolver {}

impl Z3PolicySolver {
    pub fn new() -> Self {
        Z3PolicySolver {
            ctx: Context::new(&Config::new()),
        }
    }

    pub fn verify(
        &self,
        action_field: &str,
        capability: &str,
        target: &str,
        amount: f64,
        constraints: &[Constraint],
    ) -> Result<(), String> {
        let solver = Solver::new(&self.ctx);

        let a_int = Int::new_const(&self.ctx, "action");
        let c_int = Int::new_const(&self.ctx, "capability");
        let t_int = Int::new_const(&self.ctx, "target");
        let amt_int = Int::new_const(&self.ctx, "amount");

        let mut sym_table: HashMap<String, i64> = HashMap::new();

        let mut intern = |s: &str| -> i64 {
            let id = sym_table.len() as i64;
            *sym_table.entry(s.to_string()).or_insert(id)
        };

        let a_val = intern(action_field);
        let c_val = intern(capability);
        let t_val = intern(target);

        solver.assert(&a_int._eq(&Int::from_i64(&self.ctx, a_val)));
        solver.assert(&c_int._eq(&Int::from_i64(&self.ctx, c_val)));
        solver.assert(&t_int._eq(&Int::from_i64(&self.ctx, t_val)));
        solver.assert(&amt_int._eq(&Int::from_i64(&self.ctx, amount as i64)));

        for c in constraints {
            let assertion = self.constraint_to_z3(c, &a_int, &c_int, &t_int, &amt_int, &mut sym_table);
            solver.assert(&assertion);
        }

        match solver.check() {
            SatResult::Sat => Ok(()),
            SatResult::Unsat => {
                let reasons: Vec<String> = constraints
                    .iter()
                    .map(|c| format!("rule '{}' may be violated", c.rule))
                    .collect();
                Err(format!("Blocked by Z3 solver: {}", reasons.join(", ")))
            }
            SatResult::Unknown => Err("Z3 solver returned Unknown (timeout)".to_string()),
        }
    }

    fn str_to_int(&self, s: &str, table: &mut HashMap<String, i64>) -> i64 {
        let id = table.len() as i64;
        *table.entry(s.to_string()).or_insert(id)
    }

    fn constraint_to_z3<'a>(
        &'a self,
        c: &Constraint,
        a_var: &'a Int<'a>,
        c_var: &'a Int<'a>,
        t_var: &'a Int<'a>,
        amt_var: &'a Int<'a>,
        sym_table: &mut HashMap<String, i64>,
    ) -> Bool<'a> {
        let field_var = match c.field.as_str() {
            ACTION => a_var,
            CAPABILITY => c_var,
            TARGET => t_var,
            _ => amt_var,
        };

        match c.operator.as_str() {
            "==" => {
                if let Some(s) = c.value.as_str() {
                    let val = self.str_to_int(s, sym_table);
                    field_var._eq(&Int::from_i64(&self.ctx, val))
                } else if let Some(n) = c.value.as_f64() {
                    amt_var._eq(&Int::from_i64(&self.ctx, n as i64))
                } else {
                    Bool::from_bool(&self.ctx, true)
                }
            }
            "!=" => {
                if let Some(s) = c.value.as_str() {
                    let val = self.str_to_int(s, sym_table);
                    field_var._eq(&Int::from_i64(&self.ctx, val)).not()
                } else {
                    Bool::from_bool(&self.ctx, true)
                }
            }
            "<=" => {
                if let Some(n) = c.value.as_f64() {
                    amt_var.le(&Int::from_i64(&self.ctx, n as i64))
                } else {
                    Bool::from_bool(&self.ctx, true)
                }
            }
            ">=" => {
                if let Some(n) = c.value.as_f64() {
                    amt_var.ge(&Int::from_i64(&self.ctx, n as i64))
                } else {
                    Bool::from_bool(&self.ctx, true)
                }
            }
            "in" => {
                if let Some(arr) = c.value.as_array() {
                    let or_terms: Vec<Bool> = arr
                        .iter()
                        .filter_map(|item| item.as_str())
                        .map(|s| {
                            let val = self.str_to_int(s, sym_table);
                            field_var._eq(&Int::from_i64(&self.ctx, val))
                        })
                        .collect();
                    if or_terms.is_empty() {
                        Bool::from_bool(&self.ctx, true)
                    } else {
                        let refs: Vec<&Bool> = or_terms.iter().collect();
                        let mut iter = refs.into_iter();
                        let first = iter.next().unwrap();
                        iter.fold(first.clone(), |acc, term| Bool::or(&self.ctx, &[&acc, term]))
                    }
                } else {
                    Bool::from_bool(&self.ctx, true)
                }
            }
            "not_in" => {
                if let Some(arr) = c.value.as_array() {
                    let and_terms: Vec<Bool> = arr
                        .iter()
                        .filter_map(|item| item.as_str())
                        .map(|s| {
                            let val = self.str_to_int(s, sym_table);
                            field_var._eq(&Int::from_i64(&self.ctx, val)).not()
                        })
                        .collect();
                    if and_terms.is_empty() {
                        Bool::from_bool(&self.ctx, true)
                    } else {
                        let refs: Vec<&Bool> = and_terms.iter().collect();
                        let mut iter = refs.into_iter();
                        let first = iter.next().unwrap();
                        iter.fold(first.clone(), |acc, term| Bool::and(&self.ctx, &[&acc, term]))
                    }
                } else {
                    Bool::from_bool(&self.ctx, true)
                }
            }
            _ => Bool::from_bool(&self.ctx, true),
        }
    }
}

impl Default for Z3PolicySolver {
    fn default() -> Self {
        Self::new()
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_z3_verify_passes() {
        let solver = Z3PolicySolver::new();
        let constraints = vec![Constraint {
            field: "action".to_string(),
            operator: "==".to_string(),
            value: serde_json::json!("financial:read"),
            rule: "must_be_read".to_string(),
        }];
        let result = solver.verify("financial:read", "financial:read", "/x", 0.0, &constraints);
        assert!(result.is_ok());
    }

    #[test]
    fn test_z3_verify_amount_constraint() {
        let solver = Z3PolicySolver::new();
        let constraints = vec![Constraint {
            field: "amount".to_string(),
            operator: "<=".to_string(),
            value: serde_json::json!(1000),
            rule: "max_amount".to_string(),
        }];
        assert!(solver.verify("read", "read", "/x", 500.0, &constraints).is_ok());
        assert!(solver.verify("read", "read", "/x", 1500.0, &constraints).is_err());
    }

    #[test]
    fn test_z3_not_in() {
        let solver = Z3PolicySolver::new();
        let constraints = vec![Constraint {
            field: "action".to_string(),
            operator: "not_in".to_string(),
            value: serde_json::json!(["dangerous:write", "dangerous:delete"]),
            rule: "blocked".to_string(),
        }];
        assert!(solver.verify("safe:read", "r", "/x", 0.0, &constraints).is_ok());
        assert!(solver.verify("dangerous:write", "w", "/x", 0.0, &constraints).is_err());
    }

    #[test]
    fn test_z3_inequality() {
        let solver = Z3PolicySolver::new();
        let constraints = vec![Constraint {
            field: "capability".to_string(),
            operator: "!=".to_string(),
            value: serde_json::json!("admin:escalate"),
            rule: "no_escalation".to_string(),
        }];
        assert!(solver.verify("read", "normal:read", "/x", 0.0, &constraints).is_ok());
        assert!(solver.verify("read", "admin:escalate", "/x", 0.0, &constraints).is_err());
    }
}
