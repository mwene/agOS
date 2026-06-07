use serde_json::Value;
use std::sync::Arc;
use wasmtime::{Engine, Module, Store};

use crate::kernel::agOSKernel;
use crate::uri::RoboticsResult;

const DEFAULT_FUEL_PER_EXECUTION: u64 = 200_000;
const MAX_MEMORY_BYTES: u64 = 10 * 65536;

pub struct WasmHardwareDriver {
    engine: Engine,
    wasm_bytes: Vec<u8>,
    driver_name: String,
}

impl WasmHardwareDriver {
    pub fn new(wasm_bytes: Vec<u8>, driver_name: &str) -> Result<Self, String> {
        let mut config = wasmtime::Config::new();
        config.consume_fuel(true);
        config.memory_reservation(MAX_MEMORY_BYTES);
        let engine = Engine::new(&config).map_err(|e| format!("WasmHardwareDriver engine: {}", e))?;

        // Validate that the WASM bytes compile before storing
        let _module = Module::new(&engine, &wasm_bytes)
            .map_err(|e| format!("WasmHardwareDriver module compile: {}", e))?;

        Ok(WasmHardwareDriver {
            engine,
            wasm_bytes: wasm_bytes.to_vec(),
            driver_name: driver_name.to_string(),
        })
    }

    pub fn module(&self) -> Result<Module, String> {
        Module::new(&self.engine, &self.wasm_bytes)
            .map_err(|e| format!("Module re-compile: {}", e))
    }

    pub fn sense(
        &self,
        sensor_id: &str,
        _kernel: &Arc<agOSKernel>,
        _pid: &str,
    ) -> Result<RoboticsResult, String> {
        let module = self.module()?;
        let mut store = Store::new(&self.engine, ());
        store
            .set_fuel(DEFAULT_FUEL_PER_EXECUTION)
            .map_err(|e| format!("Fuel set: {}", e))?;

        let instance = wasmtime::Instance::new(&mut store, &module, &[])
            .map_err(|e| format!("Instantiate: {}", e))?;

        let sense_fn = instance
            .get_typed_func::<(i32, i32), i32>(&mut store, "sense")
            .map_err(|e| format!("Function 'sense' not found: {}", e))?;

        let mem = instance
            .get_export(&mut store, "memory")
            .and_then(|e| e.into_memory())
            .ok_or_else(|| "No memory export".to_string())?;

        let alloc = instance
            .get_typed_func::<(i32,), i32>(&mut store, "alloc")
            .map_err(|e| format!("Function 'alloc' not found: {}", e))?;

        let input_json = serde_json::json!({"device_id": sensor_id});
        let input_str = serde_json::to_string(&input_json)
            .map_err(|e| format!("Input serialize: {}", e))?;
        let input_len = input_str.len() as i32;

        let input_ptr = alloc
            .call(&mut store, (input_len,))
            .map_err(|e| format!("Alloc: {}", e))?;

        let addr: usize = input_ptr.try_into().map_err(|_| "ptr conversion")?;
        mem.write(&mut store, addr, input_str.as_bytes())
            .map_err(|e| format!("Memory write: {}", e))?;

        let output_len = sense_fn
            .call(&mut store, (input_ptr, input_len))
            .map_err(|e| format!("WASM sense: {}", e))?;

        if output_len <= 0 {
            return Ok(RoboticsResult {
                output: vec![format!("{}:sense({}) -> OK (no output)", self.driver_name, sensor_id)],
                tokens_used: 1,
                driver: self.driver_name.clone(),
                mode: "SYS_SENSE".to_string(),
            });
        }

        let result_buf = instance
            .get_export(&mut store, "result_buffer_ptr")
            .and_then(|e| e.into_global())
            .and_then(|g| g.get(&mut store).i32());

        let result_ptr = match result_buf {
            Some(p) if p != 0 => p as usize,
            _ => return Ok(RoboticsResult {
                output: vec![format!("{}:sense({}) -> OK", self.driver_name, sensor_id)],
                tokens_used: 1,
                driver: self.driver_name.clone(),
                mode: "SYS_SENSE".to_string(),
            }),
        };

        let mut buf = vec![0u8; output_len as usize];
        mem.read(&store, result_ptr, &mut buf)
            .map_err(|e| format!("Memory read: {}", e))?;

        let output_str = String::from_utf8(buf).map_err(|e| format!("UTF-8: {}", e))?;
        let result_value: Value =
            serde_json::from_str(&output_str).map_err(|e| format!("JSON parse: {}", e))?;

        Ok(RoboticsResult {
            output: vec![format!("{}:sense({}) -> {}", self.driver_name, sensor_id, result_value)],
            tokens_used: 1,
            driver: self.driver_name.clone(),
            mode: "SYS_SENSE".to_string(),
        })
    }

    pub fn actuate(
        &self,
        actuator_id: &str,
        command: &Value,
        _kernel: &Arc<agOSKernel>,
        _pid: &str,
    ) -> Result<RoboticsResult, String> {
        let module = self.module()?;
        let mut store = Store::new(&self.engine, ());
        store
            .set_fuel(DEFAULT_FUEL_PER_EXECUTION)
            .map_err(|e| format!("Fuel set: {}", e))?;

        let instance = wasmtime::Instance::new(&mut store, &module, &[])
            .map_err(|e| format!("Instantiate: {}", e))?;

        let actuate_fn = instance
            .get_typed_func::<(i32, i32), i32>(&mut store, "actuate")
            .map_err(|e| format!("Function 'actuate' not found: {}", e))?;

        let mem = instance
            .get_export(&mut store, "memory")
            .and_then(|e| e.into_memory())
            .ok_or_else(|| "No memory export".to_string())?;

        let alloc = instance
            .get_typed_func::<(i32,), i32>(&mut store, "alloc")
            .map_err(|e| format!("Function 'alloc' not found: {}", e))?;

        let input_json = serde_json::json!({"device_id": actuator_id, "command": command});
        let input_str = serde_json::to_string(&input_json)
            .map_err(|e| format!("Input serialize: {}", e))?;
        let input_len = input_str.len() as i32;

        let input_ptr = alloc
            .call(&mut store, (input_len,))
            .map_err(|e| format!("Alloc: {}", e))?;

        let addr: usize = input_ptr.try_into().map_err(|_| "ptr conversion")?;
        mem.write(&mut store, addr, input_str.as_bytes())
            .map_err(|e| format!("Memory write: {}", e))?;

        let output_len = actuate_fn
            .call(&mut store, (input_ptr, input_len))
            .map_err(|e| format!("WASM actuate: {}", e))?;

        Ok(RoboticsResult {
            output: vec![format!("{}:actuate({}, {}) -> OK (len={})", self.driver_name, actuator_id, command, output_len)],
            tokens_used: 1,
            driver: self.driver_name.clone(),
            mode: "SYS_ACTUATE".to_string(),
        })
    }
}

impl Default for WasmHardwareDriver {
    fn default() -> Self {
        WasmHardwareDriver::new(vec![], "default").expect("Default WasmHardwareDriver")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::agOSKernel;
    use crate::umi::UnifiedModelInterface;

    const SENSOR_WAT: &str = r#"
    (module
        (memory (export "memory") 1)
        (global (export "result_buffer_ptr") (mut i32) (i32.const 0))
        (func (export "alloc") (param $size i32) (result i32)
            i32.const 0
        )
        (func (export "sense") (param $input_ptr i32) (param $input_len i32) (result i32)
            i32.const 0
        )
        (func (export "actuate") (param $input_ptr i32) (param $input_len i32) (result i32)
            i32.const 0
        )
    )
    "#;

    #[test]
    fn test_wasm_hardware_driver_creation() {
        let wasm_bytes = wat::parse_str(SENSOR_WAT).expect("WAT parse");
        let driver = WasmHardwareDriver::new(wasm_bytes, "TestWasmDriver");
        assert!(driver.is_ok());
        let driver = driver.unwrap();
        assert_eq!(driver.driver_name, "TestWasmDriver");
    }

    #[test]
    fn test_wasm_hardware_driver_sense() {
        let wasm_bytes = wat::parse_str(SENSOR_WAT).expect("WAT parse");
        let driver = WasmHardwareDriver::new(wasm_bytes, "TestWasmDriver").unwrap();
        let umi = UnifiedModelInterface::new();
        let kernel = Arc::new(agOSKernel::new(umi, "127.0.0.1:0", None));

        let result = driver.sense("sensor_01", &kernel, "test-pid");
        assert!(result.is_ok());
        let r = result.unwrap();
        assert_eq!(r.mode, "SYS_SENSE");
        assert_eq!(r.driver, "TestWasmDriver");
    }

    #[test]
    fn test_wasm_hardware_driver_actuate() {
        let wasm_bytes = wat::parse_str(SENSOR_WAT).expect("WAT parse");
        let driver = WasmHardwareDriver::new(wasm_bytes, "TestWasmDriver").unwrap();
        let umi = UnifiedModelInterface::new();
        let kernel = Arc::new(agOSKernel::new(umi, "127.0.0.1:0", None));

        let result = driver.actuate("servo_01", &serde_json::json!({"angle": 90.0}), &kernel, "test-pid");
        assert!(result.is_ok());
        let r = result.unwrap();
        assert_eq!(r.mode, "SYS_ACTUATE");
        assert_eq!(r.driver, "TestWasmDriver");
    }
}
