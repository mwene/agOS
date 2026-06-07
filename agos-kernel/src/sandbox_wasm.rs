use serde_json::Value;
use std::sync::Arc;
use wasmtime::{Engine, Module, Store};

use crate::kernel::agOSKernel;

const DEFAULT_FUEL_PER_EXECUTION: u64 = 100_000;
const MAX_MEMORY_BYTES: u64 = 10 * 65536;

pub struct WasmSandbox {
    engine: Engine,
}

impl WasmSandbox {
    pub fn new() -> Result<Self, String> {
        let mut config = wasmtime::Config::new();
        config.consume_fuel(true);
        config.memory_reservation(MAX_MEMORY_BYTES);
        let engine = Engine::new(&config).map_err(|e| format!("Engine init: {}", e))?;
        Ok(WasmSandbox { engine })
    }

    pub fn execute_tool(
        &self,
        wasm_bytes: &[u8],
        input: &Value,
        _kernel: &Arc<agOSKernel>,
        _pid: &str,
        fuel_quota: u64,
    ) -> Result<Value, String> {
        let module = Module::new(&self.engine, wasm_bytes)
            .map_err(|e| format!("Module compile: {}", e))?;

        let mut store = Store::new(&self.engine, ());
        store
            .set_fuel(fuel_quota.min(DEFAULT_FUEL_PER_EXECUTION))
            .map_err(|e| format!("Fuel set: {}", e))?;

        let instance = wasmtime::Instance::new(&mut store, &module, &[])
            .map_err(|e| format!("Instantiate: {}", e))?;

        let run = instance
            .get_typed_func::<(i32, i32), i32>(&mut store, "run")
            .map_err(|e| format!("Function 'run' not found: {}", e))?;

        let input_json = serde_json::to_string(input).map_err(|e| format!("Input serialize: {}", e))?;

        let mem = instance
            .get_export(&mut store, "memory")
            .and_then(|e| e.into_memory())
            .ok_or_else(|| "No memory export".to_string())?;

        let alloc = instance
            .get_typed_func::<(i32,), i32>(&mut store, "alloc")
            .map_err(|e| format!("Function 'alloc' not found: {}", e))?;

        let input_len = input_json.len() as i32;
        let input_ptr = alloc
            .call(&mut store, (input_len,))
            .map_err(|e| format!("Alloc: {}", e))?;

        let addr: usize = input_ptr.try_into().map_err(|_| "ptr conversion")?;
        mem.write(&mut store, addr, input_json.as_bytes())
            .map_err(|e| format!("Memory write: {}", e))?;

        let output_len = run
            .call(&mut store, (input_ptr, input_len))
            .map_err(|e| format!("WASM run: {}", e))?;

        if output_len <= 0 {
            return Ok(serde_json::json!({"result": output_len}));
        }

        let result_buf = instance
            .get_export(&mut store, "result_buffer_ptr")
            .and_then(|e| e.into_global())
            .and_then(|g| g.get(&mut store).i32());

        let result_ptr = match result_buf {
            Some(p) if p != 0 => p as usize,
            _ => return Ok(serde_json::json!({"result": output_len})),
        };

        let mut buf = vec![0u8; output_len as usize];
        mem.read(&store, result_ptr, &mut buf)
            .map_err(|e| format!("Memory read: {}", e))?;

        let output_str = String::from_utf8(buf).map_err(|e| format!("UTF-8: {}", e))?;
        let result: Value =
            serde_json::from_str(&output_str).map_err(|e| format!("JSON parse: {}", e))?;

        Ok(result)
    }
}

impl Default for WasmSandbox {
    fn default() -> Self {
        WasmSandbox::new().expect("Default WasmSandbox")
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use crate::kernel::agOSKernel;
    use crate::umi::UnifiedModelInterface;

    const ADD_WAT: &str = r#"
    (module
        (memory (export "memory") 1)
        (global (export "result_buffer_ptr") (mut i32) (i32.const 0))
        (func (export "alloc") (param $size i32) (result i32)
            i32.const 0
        )
        (func (export "run") (param $input_ptr i32) (param $input_len i32) (result i32)
            i32.const 42
        )
    )
    "#;

    #[test]
    fn test_wasm_sandbox_creation() {
        let sandbox = WasmSandbox::new();
        assert!(sandbox.is_ok());
    }

    #[test]
    fn test_wasm_execute_simple() {
        let sandbox = WasmSandbox::new().unwrap();
        let umi = UnifiedModelInterface::new();
        let kernel = Arc::new(agOSKernel::new(umi, "127.0.0.1:0", None));

        let wasm_bytes = wat::parse_str(ADD_WAT).expect("WAT parse");
        let input = serde_json::json!({"a": 1, "b": 2});

        let result = sandbox
            .execute_tool(&wasm_bytes, &input, &kernel, "test-pid", 100_000);

        assert!(result.is_ok());
        let val = result.unwrap();
        assert_eq!(val["result"], 42);
    }
}
