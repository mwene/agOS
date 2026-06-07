use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::{Arc, Mutex};
use uuid::Uuid;

use crate::kernel::agOSKernel;
use crate::umi::UMIResult;

const MAX_CLAIRIFICATION_ROUNDS: u32 = 3;

#[derive(Debug, Clone, PartialEq, Serialize, Deserialize)]
pub enum SenseModality {
    Text,
    Image,
    Audio,
    Video,
    Haptic,
    Olfactory,
    Gustatory,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SensoryInput {
    pub modality: SenseModality,
    pub data: String,
    pub timestamp: String,
    pub metadata: Value,
}

/// Categorises the kind of figure of speech or ambiguity detected.
#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum FigureOfSpeechKind {
    Metaphor,
    Idiom,
    Sarcasm,
    Hyperbole,
    Analogy,
    Euphemism,
    Understatement,
    CulturalReference,
    CasualPhrasing,
    AmbiguousReference,
    VoiceToneMismatch,
}

/// Describes a single ambiguous phrase or figure of speech found in a goal.
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SemanticAmbiguity {
    pub phrase: String,
    pub figure_kind: FigureOfSpeechKind,
    pub possible_literal_meanings: Vec<String>,
    pub modality_hint: Option<SenseModality>,
    pub resolved_by: String, // "heuristic", "umi", "user", or ""
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct EmbodimentGoal {
    pub id: String,
    pub original_statement: String,
    pub supporting_inputs: Vec<SensoryInput>,
    pub parsed_intent: String,
    pub detected_ambiguities: Vec<SemanticAmbiguity>,
    pub clarification_rounds: u32,
    pub needs_clarification: bool,
    pub status: GoalStatus,
    pub created_at: String,
    pub updated_at: String,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum GoalStatus {
    Received,
    Understanding,
    NeedsClarification,
    Planning,
    Executing,
    Completed,
    Failed(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PipelineTask {
    pub id: String,
    pub description: String,
    pub agent_pid: Option<String>,
    pub status: TaskStatus,
    pub result: Option<Value>,
}

#[derive(Debug, Clone, Serialize, Deserialize, PartialEq)]
pub enum TaskStatus {
    Pending,
    Running,
    Succeeded(Value),
    Failed(String),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ActionPlan {
    pub goal_id: String,
    pub tasks: Vec<PipelineTask>,
    pub requires_clarification: Vec<String>,
    pub suggested_apps: Vec<String>,
}

pub struct EmbodimentLayer {
    goals: Mutex<HashMap<String, EmbodimentGoal>>,
    active_plans: Mutex<HashMap<String, ActionPlan>>,
}

impl Default for EmbodimentLayer {
    fn default() -> Self {
        Self::new()
    }
}

impl EmbodimentLayer {
    pub fn new() -> Self {
        EmbodimentLayer {
            goals: Mutex::new(HashMap::new()),
            active_plans: Mutex::new(HashMap::new()),
        }
    }

    /// Detects figures of speech and ambiguous phrasing in a goal statement.
    ///
    /// Uses a multi-pass strategy:
    /// 1. Heuristic pattern matching for common English figures of speech.
    /// 2. Extra scrutiny for voice/text modalities (most likely to contain casual/figurative language).
    /// 3. Returns detected ambiguities; full resolution is deferred to UMI and clarification dialogue.
    pub fn detect_figures_of_speech(
        statement: &str,
        support_inputs: &[SensoryInput],
    ) -> Vec<SemanticAmbiguity> {
        let mut ambiguities = Vec::new();
        let lower = statement.to_lowercase();

        // ── known English figures of speech ──────────────────────────────────
        macro_rules! figure {
            ($phrase:expr, $kind:ident, [$($m:expr),+ $(,)?]) => {
                if lower.contains($phrase) {
                    ambiguities.push(SemanticAmbiguity {
                        phrase: $phrase.to_string(),
                        figure_kind: FigureOfSpeechKind::$kind,
                        possible_literal_meanings: vec![$($m.to_string()),+],
                        modality_hint: None,
                        resolved_by: String::new(),
                    });
                }
            };
        }

        figure!("pull the trigger", Idiom, ["make a decision", "execute", "start the process"]);
        figure!("hit the ground running", Idiom, ["start quickly at full pace"]);
        figure!("ballpark", CasualPhrasing, ["approximate", "rough estimate"]);
        figure!("think outside the box", Idiom, ["be creative", "explore unconventional ideas"]);
        figure!("get the ball rolling", Idiom, ["start", "initiate the process"]);
        figure!("low-hanging fruit", Metaphor, ["easy tasks", "quick wins"]);
        figure!("drill down", Metaphor, ["investigate deeply", "analyse in detail"]);
        figure!("move the needle", Metaphor, ["make measurable progress"]);
        figure!("burning platform", Metaphor, ["urgent crisis requiring immediate action"]);
        figure!("circle back", Idiom, ["revisit later", "follow up"]);
        figure!("touch base", Idiom, ["check in", "coordinate briefly"]);
        figure!("ducks in a row", Idiom, ["get organised", "prepare thoroughly"]);
        figure!("big picture", Idiom, ["overall strategy", "long-term view"]);
        figure!("on the same page", Idiom, ["align understanding", "agree"]);
        figure!("game plan", Metaphor, ["strategy", "structured approach"]);
        figure!("deep dive", Metaphor, ["thorough examination", "detailed analysis"]);
        figure!("boil the ocean", Hyperbole, ["attempt something impossibly large"]);
        figure!("shoot the moon", Hyperbole, ["aim for the best possible outcome"]);
        figure!("piece of cake", Idiom, ["very easy", "trivial"]);
        figure!("once in a blue moon", Hyperbole, ["rarely", "infrequently"]);

        // ── modality-aware casual phrasing detection ────────────────────────
        let has_voice_text = support_inputs.iter().any(|s| {
            matches!(s.modality, SenseModality::Text | SenseModality::Audio)
        });

        if has_voice_text {
            let has_audio = support_inputs.iter().any(|s| s.modality == SenseModality::Audio);
            let modality_for_hint = |is_audio: bool| {
                if is_audio { SenseModality::Audio } else { SenseModality::Text }
            };

            let casual = [
                ("kind of", "kind of / sort of"),
                ("sort of", "kind of / sort of"),
                ("whatever", "whatever"),
                ("stuff", "stuff / things"),
                ("things", "stuff / things"),
                ("do something", "do something (vague)"),
                ("something like that", "something like that"),
            ];
            for (pat, label) in &casual {
                if lower.contains(pat) && !ambiguities.iter().any(|a| a.phrase == *label) {
                    ambiguities.push(SemanticAmbiguity {
                        phrase: label.to_string(),
                        figure_kind: FigureOfSpeechKind::CasualPhrasing,
                        possible_literal_meanings: vec!["please specify precisely".to_string()],
                        modality_hint: Some(modality_for_hint(has_audio)),
                        resolved_by: String::new(),
                    });
                }
            }
        }

        ambiguities
    }

    pub fn submit_goal(
        &self,
        statement: &str,
        supporting_inputs: Vec<SensoryInput>,
        kernel: &Arc<agOSKernel>,
    ) -> EmbodimentGoal {
        let id = format!("goal-{}", &Uuid::new_v4().to_string()[..12]);
        let now = Utc::now().to_rfc3339();

        let ambiguities = Self::detect_figures_of_speech(statement, &supporting_inputs);

        let goal = EmbodimentGoal {
            id: id.clone(),
            original_statement: statement.to_string(),
            supporting_inputs,
            parsed_intent: String::new(),
            detected_ambiguities: ambiguities,
            clarification_rounds: 0,
            needs_clarification: false,
            status: GoalStatus::Received,
            created_at: now.clone(),
            updated_at: now,
        };

        self.goals.lock().unwrap().insert(id.clone(), goal.clone());

        kernel.memory.log_episodic(
            "embodiment",
            "GOAL_SUBMITTED",
            serde_json::json!({
                "goal_id": &id,
                "statement": statement,
            }),
            None,
        );

        goal
    }

    /// Consults UMI to resolve detected figures of speech before asking the user.
    ///
    /// Returns enriched ambiguities where UMI-confident meanings are marked
    /// `resolved_by: "umi"`. The task planner then uses the resolved meaning
    /// directly; only truly unresolvable ambiguities reach the user.
    async fn resolve_with_umi(
        kernel: &Arc<agOSKernel>,
        statement: &str,
        ambiguities: &[SemanticAmbiguity],
    ) -> Vec<SemanticAmbiguity> {
        if ambiguities.is_empty() {
            return Vec::new();
        }

        let figure_list: Vec<String> = ambiguities
            .iter()
            .map(|a| {
                format!(
                    "- \"{}\" ({:?}) — possible meanings: {}",
                    a.phrase,
                    a.figure_kind,
                    a.possible_literal_meanings.join(", "),
                )
            })
            .collect();

        let prompt = vec![
            format!(
                "You are agOS's semantic resolver. Your job is to interpret figures of speech \
                 and ambiguous phrasing so the system can act without bothering the user.\n\n\
                 User said: \"{statement}\"\n\n\
                 Detected ambiguous phrases:\n{list}\n\n\
                 For each one:\n\
                 - If you can determine the most likely literal, actionable meaning in context, \
                 respond with: RESOLVED: \"<phrase>\" → <literal meaning>\n\
                 - If still ambiguous after your best effort, respond with: \
                 AMBIGUOUS: \"<phrase>\" → <what makes it unclear>\n\n\
                 Be decisive — the goal is to minimise user clarification rounds \
                 while avoiding incorrect actions.",
                statement = statement,
                list = figure_list.join("\n"),
            ),
            "RESOLVED / AMBIGUOUS per phrase:".to_string(),
        ];

        let result = kernel.umi.reason(&prompt, &Value::Null).await;

        let mut enriched = ambiguities.to_vec();
        for line in &result.output {
            if let Some(after) = line.strip_prefix("RESOLVED: ") {
                if let Some((phrase_raw, meaning)) = after.split_once("→") {
                    let phrase = phrase_raw.trim().trim_matches('"');
                    let meaning = meaning.trim();
                    if let Some(a) = enriched.iter_mut().find(|a| a.phrase == phrase) {
                        if !a.possible_literal_meanings.contains(&meaning.to_string()) {
                            a.possible_literal_meanings.push(meaning.to_string());
                        }
                        a.resolved_by = "umi".to_string();
                    }
                }
            }
        }

        enriched
    }

    /// Formats detected ambiguities into a context vector, marking resolution status.
    /// Items resolved by UMI are labelled [Resolved]; others [Unresolved].
    fn format_ambiguity_context(ambiguities: &[SemanticAmbiguity]) -> Vec<String> {
        ambiguities
            .iter()
            .map(|a| {
                let meanings = a.possible_literal_meanings.join(", ");
                if a.resolved_by == "umi" {
                    format!(
                        "[Resolved] \"{}\" → {}",
                        a.phrase, meanings,
                    )
                } else {
                    format!(
                        "[Unresolved {:?}] \"{}\" → could mean: {}",
                        a.figure_kind, a.phrase, meanings,
                    )
                }
            })
            .collect()
    }

    pub async fn run_pipeline(
        &self,
        goal_id: &str,
        kernel: &Arc<agOSKernel>,
    ) -> Result<ActionPlan, String> {
        let (statement, supports) = {
            let mut guard = self.goals.lock().unwrap();
            let goal = guard
                .get_mut(goal_id)
                .ok_or_else(|| format!("Goal '{}' not found", goal_id))?;
            goal.status = GoalStatus::Understanding;
            goal.updated_at = Utc::now().to_rfc3339();
            (goal.original_statement.clone(), goal.supporting_inputs.clone())
        };

        // ── detect figures of speech → try UMI resolution ───────────────────
        let heuristic = Self::detect_figures_of_speech(&statement, &supports);
        let ambiguities = Self::resolve_with_umi(kernel, &statement, &heuristic).await;
        let ambiguity_context = Self::format_ambiguity_context(&ambiguities);

        let understanding = self
            .understand_goal(&statement, &ambiguity_context, kernel)
            .await;

        {
            let mut guard = self.goals.lock().unwrap();
            let goal = guard
                .get_mut(goal_id)
                .ok_or_else(|| format!("Goal '{}' not found", goal_id))?;
            goal.parsed_intent = understanding.output.join("; ");
            goal.detected_ambiguities = ambiguities.clone();
            goal.status = GoalStatus::Planning;
            goal.updated_at = Utc::now().to_rfc3339();
        }

        let plan = self.create_plan(goal_id, &understanding, &ambiguity_context);

        {
            let mut guard = self.goals.lock().unwrap();
            let goal = guard
                .get_mut(goal_id)
                .ok_or_else(|| format!("Goal '{}' not found", goal_id))?;

            if !plan.requires_clarification.is_empty()
                && goal.clarification_rounds < MAX_CLAIRIFICATION_ROUNDS
            {
                goal.needs_clarification = true;
                goal.status = GoalStatus::NeedsClarification;
                goal.clarification_rounds += 1;
            } else {
                goal.status = GoalStatus::Executing;
            }
            goal.updated_at = Utc::now().to_rfc3339();
        }

        self.active_plans
            .lock()
            .unwrap()
            .insert(goal_id.to_string(), plan.clone());

        kernel.memory.log_episodic(
            "embodiment",
            "PIPELINE_PLAN_CREATED",
            serde_json::json!({
                "goal_id": goal_id,
                "task_count": plan.tasks.len(),
                "needs_clarification": plan.requires_clarification,
                "ambiguities_detected": ambiguity_context,
            }),
            None,
        );

        Ok(plan)
    }

    /// Sends the goal to UMI for structured task breakdown.
    ///
    /// The prompt is enhanced to:
    /// - Detect and resolve figures of speech (metaphors, idioms, hyperbole, etc.)
    /// - Pay special attention to voice/text inputs where casual phrasing is common
    /// - Return concrete actionable sub-tasks
    async fn understand_goal(
        &self,
        statement: &str,
        ambiguity_context: &[String],
        kernel: &Arc<agOSKernel>,
    ) -> UMIResult {
        let ambiguity_hint = if ambiguity_context.is_empty() {
            "No figures of speech detected.".to_string()
        } else {
            format!(
                "Detected possible figures of speech:\n{}",
                ambiguity_context.join("\n")
            )
        };

        let prompt = vec![
            format!(
                "You are the embodiment layer of agOS. Parse this user goal into actionable sub-tasks.\n\n\
                 Goal: {statement}\n\n\
                 {ambiguity_hint}\n\n\
                 INSTRUCTIONS:\n\
                 1. Identify any figures of speech, metaphors, idioms, hyperbole, sarcasm, or casual phrasing.\n\
                 2. Resolve each to its most likely literal, actionable meaning.\n\
                 3. If the goal is ambiguous, say what is unclear.\n\
                 4. Break the *resolved* intent into concrete sub-tasks an autonomous agent can execute.\n\
                 5. Pay extra attention to voice/text style — casual phrases often hide real intent.\n\n\
                 Output one sub-task per line, prefixed with '- '.",
                statement = statement,
                ambiguity_hint = ambiguity_hint,
            ),
            "- Actionable sub-tasks (resolved from figures of speech if any):".to_string(),
        ];

        kernel.umi.reason(&prompt, &Value::Null).await
    }

    fn create_plan(
        &self,
        goal_id: &str,
        understanding: &UMIResult,
        ambiguity_context: &[String],
    ) -> ActionPlan {
        let mut tasks: Vec<PipelineTask> = understanding
            .output
            .iter()
            .enumerate()
            .map(|(i, step)| PipelineTask {
                id: format!("task-{}-{}", &goal_id[..8], i),
                description: step.clone(),
                agent_pid: None,
                status: TaskStatus::Pending,
                result: None,
            })
            .collect();

        let mut requires_clarification = Vec::new();

        // ── empty tasks means UMI could not parse → need clarification ──────
        if tasks.is_empty() {
            requires_clarification.push(
                "Could not derive any actionable sub-tasks from the goal".to_string(),
            );
            tasks.push(PipelineTask {
                id: format!("task-{}-clarify", &goal_id[..8]),
                description: "Awaiting user clarification".to_string(),
                agent_pid: None,
                status: TaskStatus::Pending,
                result: None,
            });
        }

        // ── only unresolvable figures of speech trigger clarification ───────
        // Items prefixed [Resolved] were handled by UMI and are safe to ignore.
        for ambig in ambiguity_context {
            if ambig.starts_with("[Unresolved") {
                requires_clarification.push(format!("Ambiguity: {}", ambig));
            }
        }

        let suggested_apps = self.search_agent_apps();

        ActionPlan {
            goal_id: goal_id.to_string(),
            tasks,
            requires_clarification,
            suggested_apps,
        }
    }

    fn search_agent_apps(&self) -> Vec<String> {
        vec![
            "financial_auditor".to_string(),
            "accountant".to_string(),
            "data_analyzer".to_string(),
            "report_generator".to_string(),
            "scheduler".to_string(),
            "email_agent".to_string(),
        ]
    }

    pub async fn execute_plan(
        &self,
        goal_id: &str,
        kernel: &Arc<agOSKernel>,
    ) -> Result<(), String> {
        let plan = self
            .active_plans
            .lock()
            .unwrap()
            .get(goal_id)
            .ok_or_else(|| format!("No plan for goal '{}'", goal_id))?
            .clone();

        let mut updated_tasks = Vec::new();
        for task in &plan.tasks {
            if task.description == "Awaiting user clarification" {
                updated_tasks.push(task.clone());
                continue;
            }

            let manifest = serde_json::json!({
                "goal_id": goal_id,
                "task": task.description,
                "priority": "normal",
            });

            let ctx = kernel.spawn_process(manifest, Some("embodiment".to_string())).await;

            kernel.cipc.register_process(&ctx.pid, vec![]);

            kernel.memory.log_episodic(
                &ctx.pid,
                "TASK_SPAWNED",
                serde_json::json!({
                    "goal_id": goal_id,
                    "task_id": &task.id,
                    "task": &task.description,
                }),
                None,
            );

            let mut updated = task.clone();
            updated.status = TaskStatus::Running;
            updated.agent_pid = Some(ctx.pid);
            updated_tasks.push(updated);
        }

        if let Some(p) = self.active_plans.lock().unwrap().get_mut(goal_id) {
            p.tasks = updated_tasks;
        }

        if let Some(g) = self.goals.lock().unwrap().get_mut(goal_id) {
            g.status = GoalStatus::Completed;
            g.updated_at = Utc::now().to_rfc3339();
        }

        kernel.memory.log_episodic(
            "embodiment",
            "PIPELINE_EXECUTED",
            serde_json::json!({
                "goal_id": goal_id,
                "tasks_spawned": plan.tasks.len(),
            }),
            None,
        );

        Ok(())
    }

    pub async fn provide_clarification(
        &self,
        goal_id: &str,
        clarification: &str,
        kernel: &Arc<agOSKernel>,
    ) -> Result<ActionPlan, String> {
        let (statement, supports, rounds, prior_ambiguities) = {
            let mut guard = self.goals.lock().unwrap();
            let goal = guard
                .get_mut(goal_id)
                .ok_or_else(|| format!("Goal '{}' not found", goal_id))?;

            if goal.status != GoalStatus::NeedsClarification {
                return Err(format!(
                    "Goal '{}' is in '{:?}' state, expected 'NeedsClarification'",
                    goal_id, goal.status
                ));
            }

            if goal.clarification_rounds >= MAX_CLAIRIFICATION_ROUNDS {
                return Err(format!(
                    "Goal '{}' has exhausted its {} clarification rounds",
                    goal_id, MAX_CLAIRIFICATION_ROUNDS
                ));
            }

            let updated_statement =
                format!("{}. Clarification: {}", goal.original_statement, clarification);
            goal.original_statement = updated_statement.clone();
            goal.needs_clarification = false;
            goal.clarification_rounds += 1;
            goal.status = GoalStatus::Understanding;
            goal.updated_at = Utc::now().to_rfc3339();

            kernel.memory.log_episodic(
                "embodiment",
                "CLARIFICATION_PROVIDED",
                serde_json::json!({
                    "goal_id": goal_id,
                    "clarification": clarification,
                    "round": goal.clarification_rounds,
                    "updated_statement": &updated_statement,
                    "prior_ambiguities": goal.detected_ambiguities,
                }),
                None,
            );

            (
                updated_statement,
                goal.supporting_inputs.clone(),
                goal.clarification_rounds,
                goal.detected_ambiguities.clone(),
            )
        };

        // ── re-detect figures of speech with new context ────────────────────
        // The user's clarification is treated as resolving all prior ambiguities.
        let resolved_phrases: Vec<&str> =
            prior_ambiguities.iter().map(|a| a.phrase.as_str()).collect();

        let mut new_ambiguities = Self::detect_figures_of_speech(&statement, &supports);
        new_ambiguities.retain(|a| !resolved_phrases.contains(&a.phrase.as_str()));

        // Ask UMI to resolve any newly-introduced figures of speech
        let new_ambiguities =
            Self::resolve_with_umi(kernel, &statement, &new_ambiguities).await;

        // Build full context (resolved + unresolved) for UMI; create_plan filters
        let ambiguity_context = Self::format_ambiguity_context(&new_ambiguities);

        let understanding = self
            .understand_goal(&statement, &ambiguity_context, kernel)
            .await;

        let plan = self.create_plan(goal_id, &understanding, &ambiguity_context);

        {
            let mut guard = self.goals.lock().unwrap();
            let goal = guard
                .get_mut(goal_id)
                .ok_or_else(|| format!("Goal '{}' not found", goal_id))?;
            goal.parsed_intent = understanding.output.join("; ");
            goal.detected_ambiguities = new_ambiguities;
            goal.updated_at = Utc::now().to_rfc3339();

            if !plan.requires_clarification.is_empty() && rounds < MAX_CLAIRIFICATION_ROUNDS {
                goal.needs_clarification = true;
                goal.status = GoalStatus::NeedsClarification;
            } else {
                goal.status = GoalStatus::Executing;
            }
        }

        self.active_plans
            .lock()
            .unwrap()
            .insert(goal_id.to_string(), plan.clone());

        Ok(plan)
    }

    pub fn process_sensory_input(
        &self,
        modality: SenseModality,
        data: &str,
        metadata: Value,
    ) -> SensoryInput {
        SensoryInput {
            modality,
            data: data.to_string(),
            timestamp: Utc::now().to_rfc3339(),
            metadata,
        }
    }

    pub fn get_goal(&self, goal_id: &str) -> Option<EmbodimentGoal> {
        self.goals.lock().unwrap().get(goal_id).cloned()
    }

    pub fn get_plan(&self, goal_id: &str) -> Option<ActionPlan> {
        self.active_plans.lock().unwrap().get(goal_id).cloned()
    }

    pub fn list_goals(&self) -> Vec<EmbodimentGoal> {
        let mut goals: Vec<_> = self.goals.lock().unwrap().values().cloned().collect();
        goals.sort_by(|a, b| b.created_at.cmp(&a.created_at));
        goals
    }

    pub fn available_sensors(&self) -> Vec<SenseModality> {
        vec![
            SenseModality::Text,
            SenseModality::Image,
            SenseModality::Audio,
            SenseModality::Video,
            SenseModality::Haptic,
            SenseModality::Olfactory,
            SenseModality::Gustatory,
        ]
    }

    pub fn list_agent_apps(&self) -> Vec<String> {
        vec![
            "financial_auditor".to_string(),
            "accountant".to_string(),
            "data_analyzer".to_string(),
            "report_generator".to_string(),
            "scheduler".to_string(),
            "email_agent".to_string(),
        ]
    }

    #[cfg(test)]
    pub fn set_clarification_needed(&self, goal_id: &str) -> Result<(), String> {
        let mut guard = self.goals.lock().unwrap();
        let goal = guard
            .get_mut(goal_id)
            .ok_or_else(|| format!("Goal '{}' not found", goal_id))?;
        goal.needs_clarification = true;
        goal.status = GoalStatus::NeedsClarification;
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::umi::UnifiedModelInterface;

    fn test_kernel() -> Arc<agOSKernel> {
        Arc::new(agOSKernel::new(UnifiedModelInterface::new(), "127.0.0.1:0"))
    }

    #[test]
    fn test_embodiment_layer_creation() {
        let layer = EmbodimentLayer::new();
        let sensors = layer.available_sensors();
        assert_eq!(sensors.len(), 7);
    }

    #[test]
    fn test_submit_goal() {
        let layer = EmbodimentLayer::new();
        let kernel = test_kernel();
        let goal = layer.submit_goal(
            "I want to analyze my spending habits from last month",
            vec![],
            &kernel,
        );
        assert!(goal.id.starts_with("goal-"));
        assert_eq!(goal.status, GoalStatus::Received);
    }

    #[test]
    fn test_submit_goal_with_sensory_input() {
        let layer = EmbodimentLayer::new();
        let kernel = test_kernel();

        let image_input = layer.process_sensory_input(
            SenseModality::Image,
            "base64_encoded_image_data",
            serde_json::json!({"format": "png", "width": 1920}),
        );
        let audio_input = layer.process_sensory_input(
            SenseModality::Audio,
            "base64_encoded_audio_data",
            serde_json::json!({"format": "wav", "duration_secs": 30}),
        );

        let goal = layer.submit_goal(
            "Show me a similar product to this image",
            vec![image_input, audio_input],
            &kernel,
        );
        assert_eq!(goal.supporting_inputs.len(), 2);
    }

    #[tokio::test]
    async fn test_run_pipeline_creates_plan() {
        let layer = EmbodimentLayer::new();
        let kernel = test_kernel();

        let goal = layer.submit_goal("Help me organize my monthly budget", vec![], &kernel);

        let plan = layer.run_pipeline(&goal.id, &kernel).await.unwrap();
        assert_eq!(plan.goal_id, goal.id);
        assert!(!plan.tasks.is_empty());
    }

    #[tokio::test]
    async fn test_execute_plan_spawns_tasks() {
        let layer = EmbodimentLayer::new();
        let kernel = test_kernel();

        let goal = layer.submit_goal("Generate a weekly report", vec![], &kernel);

        let _plan = layer.run_pipeline(&goal.id, &kernel).await.unwrap();

        let result = layer.execute_plan(&goal.id, &kernel).await;
        assert!(result.is_ok());

        let stored_goal = layer.get_goal(&goal.id).unwrap();
        assert_eq!(stored_goal.status, GoalStatus::Completed);
    }

    #[tokio::test]
    async fn test_provide_clarification() {
        let layer = EmbodimentLayer::new();
        let kernel = test_kernel();

        let goal = layer.submit_goal("Do something", vec![], &kernel);

        let plan1 = layer.run_pipeline(&goal.id, &kernel).await.unwrap();
        let needs_clarification = plan1.requires_clarification;

        if !needs_clarification.is_empty() {
            let plan2 = layer
                .provide_clarification(&goal.id, "I want to organize my emails", &kernel)
                .await
                .unwrap();
            assert_eq!(plan2.goal_id, goal.id);
        }
    }

    #[tokio::test]
    async fn test_clarification_dialogue_loop() {
        let layer = EmbodimentLayer::new();
        let kernel = test_kernel();

        // Normal pipeline happy path (UMI mock always produces plans)
        let goal = layer.submit_goal("organize my inbox", vec![], &kernel);
        assert_eq!(goal.status, GoalStatus::Received);

        let plan = layer.run_pipeline(&goal.id, &kernel).await.unwrap();
        assert!(
            plan.requires_clarification.is_empty(),
            "mock UMI should produce actionable tasks"
        );
        assert_eq!(
            layer.get_goal(&goal.id).unwrap().status,
            GoalStatus::Executing
        );

        // Now force NeedsClarification to test the clarification path
        layer.set_clarification_needed(&goal.id).unwrap();
        let plan2 = layer
            .provide_clarification(&goal.id, "by priority level", &kernel)
            .await
            .unwrap();
        assert!(plan2.requires_clarification.is_empty());
        let updated = layer.get_goal(&goal.id).unwrap();
        assert!(updated.original_statement.contains("Clarification:"));
        assert_eq!(updated.clarification_rounds, 1);
        assert_eq!(updated.status, GoalStatus::Executing);
    }

    #[tokio::test]
    async fn test_clarification_round_limit() {
        let layer = EmbodimentLayer::new();
        let kernel = test_kernel();

        let goal = layer.submit_goal("manage tasks", vec![], &kernel);
        let _plan = layer.run_pipeline(&goal.id, &kernel).await.unwrap();
        layer.set_clarification_needed(&goal.id).unwrap();

        // Exhaust rounds
        // Must re-set NeedsClarification before each call because the mock
        // UMI always produces executable plans, so provide_clarification
        // transitions the goal to Executing after each round.
        for c in &["narrow scope", "focus on deadlines", "add priorities"] {
            layer.set_clarification_needed(&goal.id).unwrap();
            layer
                .provide_clarification(&goal.id, c, &kernel)
                .await
                .unwrap();
        }

        let g = layer.get_goal(&goal.id).unwrap();
        assert_eq!(g.clarification_rounds, 3);

        // Round 4 should fail (rounds already at limit)
        layer.set_clarification_needed(&goal.id).unwrap();
        let err = layer
            .provide_clarification(&goal.id, "extra", &kernel)
            .await
            .unwrap_err();
        assert!(err.contains("exhausted"), "expected exhausted: {}", err);
    }

    #[tokio::test]
    async fn test_clarification_rejects_wrong_state() {
        let layer = EmbodimentLayer::new();
        let kernel = test_kernel();

        // Submit a goal, don't run_pipeline — status is "Received"
        let goal = layer.submit_goal("a task", vec![], &kernel);
        let err = layer
            .provide_clarification(&goal.id, "clarify", &kernel)
            .await
            .unwrap_err();
        assert!(
            err.contains("NeedsClarification"),
            "expected NeedsClarification error: {}",
            err
        );
    }

    #[test]
    fn test_list_goals_returns_submitted() {
        let layer = EmbodimentLayer::new();
        let kernel = test_kernel();

        layer.submit_goal("Goal A", vec![], &kernel);
        layer.submit_goal("Goal B", vec![], &kernel);

        let goals = layer.list_goals();
        assert_eq!(goals.len(), 2);
    }

    #[test]
    fn test_sensory_input_processing() {
        let layer = EmbodimentLayer::new();

        let haptic = layer.process_sensory_input(
            SenseModality::Haptic,
            "vibration_pattern_alpha",
            serde_json::json!({"intensity": 0.8, "duration_ms": 500}),
        );
        assert_eq!(haptic.modality as i32, SenseModality::Haptic as i32);

        let olfactory = layer.process_sensory_input(
            SenseModality::Olfactory,
            "gas_sensor_reading_ppm",
            serde_json::json!({"compound": "CO2", "ppm": 450}),
        );
        assert_eq!(olfactory.modality as i32, SenseModality::Olfactory as i32);

        let gustatory = layer.process_sensory_input(
            SenseModality::Gustatory,
            "ph_level_7.2",
            serde_json::json!({"substance": "water_sample"}),
        );
        assert_eq!(
            gustatory.modality as i32,
            SenseModality::Gustatory as i32
        );
    }

    // ── figure-of-speech tests ──────────────────────────────────────────────

    #[test]
    fn test_detect_idioms() {
        let ambiguities = EmbodimentLayer::detect_figures_of_speech(
            "We need to pull the trigger on the Q4 budget",
            &[],
        );
        assert!(
            ambiguities.iter().any(|a| a.phrase == "pull the trigger"),
            "should detect 'pull the trigger' idiom, got: {:#?}",
            ambiguities,
        );
    }

    #[test]
    fn test_detect_metaphors() {
        let ambiguities = EmbodimentLayer::detect_figures_of_speech(
            "Let's grab the low-hanging fruit first, then drill down on the big accounts",
            &[],
        );
        assert!(
            ambiguities.iter().any(|a| a.phrase == "low-hanging fruit"),
            "should detect 'low-hanging fruit'"
        );
        assert!(
            ambiguities.iter().any(|a| a.phrase == "drill down"),
            "should detect 'drill down'"
        );
    }

    #[test]
    fn test_detect_hyperbole() {
        let ambiguities = EmbodimentLayer::detect_figures_of_speech(
            "This task is a piece of cake, happens once in a blue moon",
            &[],
        );
        assert!(
            ambiguities.iter().any(|a| a.phrase == "piece of cake"),
            "should detect 'piece of cake'"
        );
        assert!(
            ambiguities.iter().any(|a| a.phrase == "once in a blue moon"),
            "should detect 'once in a blue moon'"
        );
    }

    #[test]
    fn test_casual_phrasing_voice_text() {
        let audio_input = SensoryInput {
            modality: SenseModality::Audio,
            data: "wake_word_detected".to_string(),
            timestamp: "now".to_string(),
            metadata: serde_json::json!({}),
        };
        let ambiguities = EmbodimentLayer::detect_figures_of_speech(
            "I want to do something with my photos, kind of organize them",
            &[audio_input],
        );
        assert!(
            ambiguities.iter().any(|a| a.figure_kind == FigureOfSpeechKind::CasualPhrasing),
            "should detect casual phrasing in voice input, got: {:#?}",
            ambiguities,
        );
    }

    #[test]
    fn test_no_false_positives_literal() {
        let ambiguities = EmbodimentLayer::detect_figures_of_speech(
            "Run a backup of the database nightly",
            &[],
        );
        assert!(
            ambiguities.is_empty(),
            "literal command should have no detected figures, got: {:#?}",
            ambiguities,
        );
    }

    #[tokio::test]
    async fn test_figures_flag_clarification_needed() {
        let layer = EmbodimentLayer::new();
        let kernel = test_kernel();

        let goal = layer.submit_goal("Let's grab the low-hanging fruit", vec![], &kernel);
        assert!(
            !goal.detected_ambiguities.is_empty(),
            "should have detected figure of speech on submission"
        );

        let plan = layer.run_pipeline(&goal.id, &kernel).await.unwrap();
        assert!(
            !plan.requires_clarification.is_empty(),
            "figures of speech should trigger clarification needed"
        );
        assert_eq!(
            layer.get_goal(&goal.id).unwrap().status,
            GoalStatus::NeedsClarification,
            "goal should be in NeedsClarification after pipeline"
        );
    }

    #[tokio::test]
    async fn test_clarify_figurative_goal() {
        let layer = EmbodimentLayer::new();
        let kernel = test_kernel();

        let goal = layer.submit_goal("Let's pull the trigger on the budget", vec![], &kernel);
        assert!(!goal.detected_ambiguities.is_empty());

        let did_need = !layer.run_pipeline(&goal.id, &kernel).await.unwrap()
            .requires_clarification.is_empty();

        if did_need {
            let plan = layer
                .provide_clarification(
                    &goal.id,
                    "I mean formally approve the spending plan for next quarter",
                    &kernel,
                )
                .await
                .unwrap();
            // After specific clarification, the ambiguity should be resolved
            let updated = layer.get_goal(&goal.id).unwrap();
            assert_eq!(updated.status, GoalStatus::Executing);
            assert!(
                plan.requires_clarification.is_empty(),
                "clarified figurative goal should produce executable plan, got: {:?}",
                plan.requires_clarification,
            );
        }
    }

    // ── UMI resolution tests ────────────────────────────────────────────────

    #[tokio::test]
    async fn test_resolve_with_umi_no_ambiguities() {
        let kernel = test_kernel();
        let result = EmbodimentLayer::resolve_with_umi(
            &kernel,
            "Run the nightly backup",
            &[],
        )
        .await;
        assert!(result.is_empty(), "no ambiguities → empty result");
    }

    #[tokio::test]
    async fn test_resolve_with_umi_unresolved_by_mock() {
        // The mock UMI always returns "reasoned: ..." which does not match
        // the "RESOLVED:" prefix, so all ambiguities stay unresolved.
        let kernel = test_kernel();
        let ambiguities = vec![
            SemanticAmbiguity {
                phrase: "pull the trigger".to_string(),
                figure_kind: FigureOfSpeechKind::Idiom,
                possible_literal_meanings: vec!["make a decision".to_string()],
                modality_hint: None,
                resolved_by: String::new(),
            },
        ];

        let result = EmbodimentLayer::resolve_with_umi(
            &kernel,
            "Let's pull the trigger on the budget",
            &ambiguities,
        )
        .await;

        assert_eq!(result.len(), 1);
        assert_eq!(
            result[0].resolved_by, "",
            "mock UMI cannot resolve figures of speech"
        );
    }

    #[tokio::test]
    async fn test_umi_resolution_integrated_in_pipeline() {
        // Verify the pipeline calls resolve_with_umi without error
        // and that unresolved ambiguities correctly trigger clarification.
        let layer = EmbodimentLayer::new();
        let kernel = test_kernel();

        let goal = layer.submit_goal("Let's pull the trigger on the budget", vec![], &kernel);
        assert!(!goal.detected_ambiguities.is_empty());

        let plan = layer.run_pipeline(&goal.id, &kernel).await.unwrap();
        // With mock UMI, ambiguities are never RESOLVED → must ask user
        assert!(
            !plan.requires_clarification.is_empty(),
            "mock UMI cannot resolve → clarification should be needed"
        );

        let stored = layer.get_goal(&goal.id).unwrap();
        assert_eq!(stored.status, GoalStatus::NeedsClarification);
        // All ambiguities should still be unresolved since mock can't parse "RESOLVED:"
        assert!(
            stored.detected_ambiguities.iter().all(|a| a.resolved_by.is_empty()),
            "all ambiguities should remain unresolved with mock UMI"
        );
    }

    #[test]
    fn test_agent_app_search() {
        let layer = EmbodimentLayer::new();
        let apps = layer.list_agent_apps();
        assert!(apps.contains(&"financial_auditor".to_string()));
        assert!(apps.contains(&"email_agent".to_string()));
    }
}
