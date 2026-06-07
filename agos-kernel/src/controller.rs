use chrono::Utc;
use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::HashMap;
use std::sync::Mutex;

/// PID gain configuration for a control loop
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PIDGains {
    pub kp: f64,
    pub ki: f64,
    pub kd: f64,
    pub saturation_max: f64,
    pub saturation_min: f64,
}

impl Default for PIDGains {
    fn default() -> Self {
        PIDGains {
            kp: 1.0,
            ki: 0.0,
            kd: 0.0,
            saturation_max: 100.0,
            saturation_min: -100.0,
        }
    }
}

/// A running PID control loop instance
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct PIDController {
    pub id: String,
    pub joint: String,
    pub gains: PIDGains,
    pub setpoint: f64,
    pub current_position: f64,
    pub integral: f64,
    pub previous_error: f64,
    pub output: f64,
    pub frequency_hz: u64,
    pub active: bool,
    pub created_at: String,
}

impl PIDController {
    pub fn new(id: String, joint: String, gains: PIDGains, frequency_hz: u64) -> Self {
        PIDController {
            id,
            joint,
            gains,
            setpoint: 0.0,
            current_position: 0.0,
            integral: 0.0,
            previous_error: 0.0,
            output: 0.0,
            frequency_hz,
            active: false,
            created_at: Utc::now().to_rfc3339(),
        }
    }

    /// Compute one control cycle: returns the control output
    pub fn compute(&mut self, measurement: f64, dt: f64) -> f64 {
        self.current_position = measurement;
        let error = self.setpoint - measurement;

        self.integral += error * dt;
        let derivative = (error - self.previous_error) / dt.max(1e-6);
        self.previous_error = error;

        let mut output = self.gains.kp * error
            + self.gains.ki * self.integral
            + self.gains.kd * derivative;

        output = output.clamp(self.gains.saturation_min, self.gains.saturation_max);
        self.output = output;
        output
    }

    pub fn set_setpoint(&mut self, setpoint: f64) {
        self.setpoint = setpoint;
        self.integral = 0.0;
        self.previous_error = 0.0;
    }
}

/// A waypoint in a trajectory
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Waypoint {
    pub joint_positions: Vec<f64>,
    pub velocity: Option<f64>,
    pub acceleration: Option<f64>,
    pub duration_secs: f64,
}

/// A computed trajectory plan
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Trajectory {
    pub id: String,
    pub joint_names: Vec<String>,
    pub waypoints: Vec<Waypoint>,
    pub current_waypoint: usize,
    pub total_duration_secs: f64,
    pub elapsed_secs: f64,
    pub active: bool,
    pub created_at: String,
}

impl Trajectory {
    pub fn new(id: String, joint_names: Vec<String>, waypoints: Vec<Waypoint>) -> Self {
        let total_duration: f64 = waypoints.iter().map(|w| w.duration_secs).sum();
        Trajectory {
            id,
            joint_names,
            waypoints,
            current_waypoint: 0,
            total_duration_secs: total_duration,
            elapsed_secs: 0.0,
            active: false,
            created_at: Utc::now().to_rfc3339(),
        }
    }

    /// Advance the trajectory by dt seconds. Returns the target joint positions at current time.
    pub fn step(&mut self, dt: f64) -> Option<Vec<f64>> {
        if !self.active || self.current_waypoint >= self.waypoints.len() {
            return None;
        }

        self.elapsed_secs += dt;

        // Find which waypoint segment we are in
        let mut accumulated = 0.0;
        for (i, wp) in self.waypoints.iter().enumerate() {
            accumulated += wp.duration_secs;
            if self.elapsed_secs <= accumulated || i == self.waypoints.len() - 1 {
                self.current_waypoint = i;
                return Some(wp.joint_positions.clone());
            }
        }
        None
    }
}

/// Safety limits for a joint or actuator
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct JointLimits {
    pub joint_name: String,
    pub position_min: f64,
    pub position_max: f64,
    pub velocity_max: f64,
    pub torque_max: f64,
    pub temperature_max: f64,
}

impl Default for JointLimits {
    fn default() -> Self {
        JointLimits {
            joint_name: "default".to_string(),
            position_min: -180.0,
            position_max: 180.0,
            velocity_max: 180.0,
            torque_max: 100.0,
            temperature_max: 85.0,
        }
    }
}

/// A safety event recorded by the monitor
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SafetyEvent {
    pub id: String,
    pub device: String,
    pub event_type: String,
    pub message: String,
    pub severity: String,
    pub timestamp: String,
    pub cleared: bool,
}

/// Safety monitor for a physical device
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct SafetyMonitor {
    pub device_id: String,
    pub limits: JointLimits,
    pub events: Vec<SafetyEvent>,
    pub e_stop_active: bool,
    pub created_at: String,
}

impl SafetyMonitor {
    pub fn new(device_id: String, limits: JointLimits) -> Self {
        SafetyMonitor {
            device_id,
            limits,
            events: Vec::new(),
            e_stop_active: false,
            created_at: Utc::now().to_rfc3339(),
        }
    }

    /// Check a joint state against limits. Returns list of violations.
    pub fn check(&mut self, position: f64, velocity: f64, torque: f64, temperature: f64) -> Vec<String> {
        let mut violations = Vec::new();

        if position < self.limits.position_min || position > self.limits.position_max {
            violations.push(format!(
                "Position {:.3} outside limits [{:.1}, {:.1}]",
                position, self.limits.position_min, self.limits.position_max
            ));
        }
        if velocity.abs() > self.limits.velocity_max {
            violations.push(format!(
                "Velocity {:.3} exceeds max {:.1}",
                velocity, self.limits.velocity_max
            ));
        }
        if torque.abs() > self.limits.torque_max {
            violations.push(format!(
                "Torque {:.3} exceeds max {:.1}",
                torque, self.limits.torque_max
            ));
        }
        if temperature > self.limits.temperature_max {
            violations.push(format!(
                "Temperature {:.1} exceeds max {:.1}",
                temperature, self.limits.temperature_max
            ));
        }

        for v in &violations {
            self.events.push(SafetyEvent {
                id: format!("safety-{}-{}", self.device_id, self.events.len()),
                device: self.device_id.clone(),
                event_type: "limit_violation".into(),
                message: v.clone(),
                severity: "warning".into(),
                timestamp: Utc::now().to_rfc3339(),
                cleared: false,
            });
        }

        violations
    }

    pub fn trigger_e_stop(&mut self, reason: &str) {
        self.e_stop_active = true;
        self.events.push(SafetyEvent {
            id: format!("safety-{}-estop", self.device_id),
            device: self.device_id.clone(),
            event_type: "emergency_stop".into(),
            message: reason.to_string(),
            severity: "critical".into(),
            timestamp: Utc::now().to_rfc3339(),
            cleared: false,
        });
    }

    pub fn clear_e_stop(&mut self) {
        self.e_stop_active = false;
    }
}

/// A named coordinate frame in the transform tree
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransformFrame {
    pub name: String,
    pub parent: Option<String>,
    pub translation: [f64; 3],
    pub rotation: [f64; 4], // quaternion (x, y, z, w)
}

/// Transform tree for multi-sensor/actuator coordinate systems
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct TransformTree {
    pub frames: HashMap<String, TransformFrame>,
}

impl Default for TransformTree {
    fn default() -> Self {
        Self::new()
    }
}

impl TransformTree {
    pub fn new() -> Self {
        let mut frames = HashMap::new();
        frames.insert(
            "world".to_string(),
            TransformFrame {
                name: "world".to_string(),
                parent: None,
                translation: [0.0, 0.0, 0.0],
                rotation: [0.0, 0.0, 0.0, 1.0],
            },
        );
        TransformTree { frames }
    }

    pub fn add_frame(&mut self, name: &str, parent: Option<&str>, translation: [f64; 3], rotation: [f64; 4]) {
        self.frames.insert(
            name.to_string(),
            TransformFrame {
                name: name.to_string(),
                parent: parent.map(|p| p.to_string()),
                translation,
                rotation,
            },
        );
    }

    pub fn list_frames(&self) -> Vec<&TransformFrame> {
        self.frames.values().collect()
    }

    pub fn get_frame(&self, name: &str) -> Option<&TransformFrame> {
        self.frames.get(name)
    }
}

/// Configuration for the controller thread
#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct ControllerConfig {
    pub control_loop_frequency_hz: u64,
    pub safety_check_interval_cycles: u64,
}

impl Default for ControllerConfig {
    fn default() -> Self {
        ControllerConfig {
            control_loop_frequency_hz: 100,
            safety_check_interval_cycles: 10,
        }
    }
}

/// Top-level hardware controller manager
pub struct ControllerManager {
    pub config: ControllerConfig,
    pub pid_controllers: Mutex<HashMap<String, PIDController>>,
    pub trajectories: Mutex<HashMap<String, Trajectory>>,
    pub safety_monitors: Mutex<HashMap<String, SafetyMonitor>>,
    pub transform_tree: Mutex<TransformTree>,
}

impl Default for ControllerManager {
    fn default() -> Self {
        Self::new()
    }
}

impl ControllerManager {
    pub fn new() -> Self {
        ControllerManager {
            config: ControllerConfig::default(),
            pid_controllers: Mutex::new(HashMap::new()),
            trajectories: Mutex::new(HashMap::new()),
            safety_monitors: Mutex::new(HashMap::new()),
            transform_tree: Mutex::new(TransformTree::new()),
        }
    }

    // ── PID Control ────────────────────────────────────────────────

    pub fn create_pid_controller(
        &self,
        joint: &str,
        gains: PIDGains,
        frequency_hz: u64,
    ) -> PIDController {
        let id = format!("pid-{}", joint);
        let ctrl = PIDController::new(id.clone(), joint.to_string(), gains, frequency_hz);
        self.pid_controllers.lock().unwrap().insert(id.clone(), ctrl.clone());
        ctrl
    }

    pub fn get_pid_controller(&self, id: &str) -> Option<PIDController> {
        self.pid_controllers.lock().unwrap().get(id).cloned()
    }

    pub fn list_pid_controllers(&self) -> Vec<PIDController> {
        self.pid_controllers.lock().unwrap().values().cloned().collect()
    }

    pub fn set_pid_setpoint(&self, id: &str, setpoint: f64) -> Result<(), String> {
        let mut ctrls = self.pid_controllers.lock().unwrap();
        let ctrl = ctrls.get_mut(id).ok_or_else(|| format!("PID controller '{}' not found", id))?;
        ctrl.set_setpoint(setpoint);
        ctrl.active = true;
        Ok(())
    }

    pub fn step_pid_controller(&self, id: &str, measurement: f64, dt: f64) -> Result<f64, String> {
        let mut ctrls = self.pid_controllers.lock().unwrap();
        let ctrl = ctrls.get_mut(id).ok_or_else(|| format!("PID controller '{}' not found", id))?;
        Ok(ctrl.compute(measurement, dt))
    }

    // ── Trajectory ─────────────────────────────────────────────────

    pub fn create_trajectory(
        &self,
        joint_names: Vec<String>,
        waypoints: Vec<Waypoint>,
    ) -> Trajectory {
        let id = format!("traj-{}", uuid::Uuid::new_v4().to_string());
        let traj = Trajectory::new(id.clone(), joint_names, waypoints);
        self.trajectories.lock().unwrap().insert(id.clone(), traj.clone());
        traj
    }

    pub fn start_trajectory(&self, id: &str) -> Result<(), String> {
        let mut trajs = self.trajectories.lock().unwrap();
        let traj = trajs.get_mut(id).ok_or_else(|| format!("Trajectory '{}' not found", id))?;
        traj.active = true;
        traj.elapsed_secs = 0.0;
        traj.current_waypoint = 0;
        Ok(())
    }

    pub fn step_trajectory(&self, id: &str, dt: f64) -> Result<Option<Vec<f64>>, String> {
        let mut trajs = self.trajectories.lock().unwrap();
        let traj = trajs.get_mut(id).ok_or_else(|| format!("Trajectory '{}' not found", id))?;
        Ok(traj.step(dt))
    }

    pub fn list_trajectories(&self) -> Vec<Trajectory> {
        self.trajectories.lock().unwrap().values().cloned().collect()
    }

    // ── Safety ─────────────────────────────────────────────────────

    pub fn create_safety_monitor(&self, device_id: &str, limits: JointLimits) -> SafetyMonitor {
        let monitor = SafetyMonitor::new(device_id.to_string(), limits);
        self.safety_monitors.lock().unwrap().insert(device_id.to_string(), monitor.clone());
        monitor
    }

    pub fn check_safety(
        &self,
        device_id: &str,
        position: f64,
        velocity: f64,
        torque: f64,
        temperature: f64,
    ) -> Result<(), Vec<String>> {
        let mut monitors = self.safety_monitors.lock().unwrap();
        let monitor = monitors.get_mut(device_id)
            .ok_or_else(|| vec![format!("No safety monitor for '{}'", device_id)])?;

        if monitor.e_stop_active {
            return Err(vec!["Emergency stop active".to_string()]);
        }

        let violations = monitor.check(position, velocity, torque, temperature);
        if violations.is_empty() {
            Ok(())
        } else {
            Err(violations)
        }
    }

    pub fn trigger_e_stop(&self, device_id: &str, reason: &str) -> Result<(), String> {
        let mut monitors = self.safety_monitors.lock().unwrap();
        let monitor = monitors.get_mut(device_id)
            .ok_or_else(|| format!("No safety monitor for '{}'", device_id))?;
        monitor.trigger_e_stop(reason);
        Ok(())
    }

    pub fn clear_e_stop(&self, device_id: &str) -> Result<(), String> {
        let mut monitors = self.safety_monitors.lock().unwrap();
        let monitor = monitors.get_mut(device_id)
            .ok_or_else(|| format!("No safety monitor for '{}'", device_id))?;
        monitor.clear_e_stop();
        Ok(())
    }

    pub fn list_safety_monitors(&self) -> Vec<SafetyMonitor> {
        self.safety_monitors.lock().unwrap().values().cloned().collect()
    }

    pub fn list_safety_events(&self, device_id: &str) -> Vec<SafetyEvent> {
        let monitors = self.safety_monitors.lock().unwrap();
        monitors.get(device_id).map(|m| m.events.clone()).unwrap_or_default()
    }

    // ── Transforms ─────────────────────────────────────────────────

    pub fn add_transform_frame(
        &self,
        name: &str,
        parent: Option<&str>,
        translation: [f64; 3],
        rotation: [f64; 4],
    ) {
        let mut tree = self.transform_tree.lock().unwrap();
        tree.add_frame(name, parent, translation, rotation);
    }

    pub fn list_transform_frames(&self) -> Vec<TransformFrame> {
        let tree = self.transform_tree.lock().unwrap();
        tree.list_frames().into_iter().cloned().collect()
    }

    pub fn get_stats(&self) -> Value {
        let pid_count = self.pid_controllers.lock().unwrap().len();
        let traj_count = self.trajectories.lock().unwrap().len();
        let monitor_count = self.safety_monitors.lock().unwrap().len();
        let frame_count = self.transform_tree.lock().unwrap().frames.len();
        serde_json::json!({
            "pid_controllers": pid_count,
            "trajectories": traj_count,
            "safety_monitors": monitor_count,
            "transform_frames": frame_count,
            "control_loop_frequency_hz": self.config.control_loop_frequency_hz,
        })
    }
}
