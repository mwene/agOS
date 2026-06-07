# URI Driver Developer Guide

This guide covers how to develop, register, and manage hardware drivers for the **Unified Robotics Interface (URI)** — the abstraction layer that decouples agOS agents from any specific sensor, actuator, or hardware protocol.

---

## Table of Contents

- [What is URI?](#what-is-uri)
- [Driver Architecture](#driver-architecture)
- [The HardwareDriver Struct](#the-hardwaredriver-struct)
- [The UnifiedRoboticsInterface Struct](#the-unifiedroboticsinterface-struct)
- [Built-in Drivers](#built-in-drivers)
- [Implementing a Built-in Driver](#implementing-a-built-in-driver)
- [Registering a Runtime Driver](#registering-a-runtime-driver)
- [URI Syscalls Reference](#uri-syscalls-reference)
- [End-to-End Walkthrough: Adding a GPIO Driver](#end-to-end-walkthrough-adding-a-gpio-driver)
- [Best Practices](#best-practices)
- [Troubleshooting](#troubleshooting)

---

## What is URI?

The **Unified Robotics Interface** is the hardware abstraction layer in agOS. Just as UMI abstracts AI model inference, URI abstracts physical hardware through a uniform syscall interface:

| Syscall | Function | Description |
|---------|----------|-------------|
| `SENSE` | `sense()` | Read a value from a sensor |
| `ACTUATE` | `actuate()` | Write a value to an actuator |
| `TRANSFORM` | `transform()` | Apply a data transform pipeline |
| `STATUS` | `status()` | Query driver/device health |

Agents call these syscalls via the REST API. The URI routes them to the appropriate hardware driver based on device-to-driver mappings.

```
  Agent A                 Agent B
    │                        │
    └──────┬─────────────────┘
           │
    ┌──────▼──────┐
    │    URI      │  ← routes to device's driver
    └──────┬──────┘
           │
    ┌──────┼──────┬──────────────┐
    │      │      │              │
    ▼      ▼      ▼              ▼
 SimTemp SimServo SimDist    Custom Driver...
 (builtin) (builtin) (builtin) (yours here)
```

---

## Driver Architecture

Drivers in URI come in two flavours:

1. **Built-in drivers** — compile-time statics embedded in `uri.rs`. Fast, always available, limited to capabilities known at compile time.
2. **Runtime drivers** — registered dynamically via REST at runtime. Can connect to any hardware accessible from the host (GPIO, I2C, SPI, serial, network).

Both types implement the same interface — the `HardwareDriver` struct — and are stored in the same registry map.

```
UnifiedRoboticsInterface
├── drivers: RwLock<HashMap<String, HardwareDriver>>
├── devices: RwLock<HashMap<String, DeviceState>>
├── transforms: RwLock<Vec<DataTransform>>
│
├── sense(device_id, driver_name?) ───→ driver.sense(device_id)
├── actuate(device_id, value, driver_name?) ───→ driver.actuate(device_id, value)
├── transform(name, data) ───→ apply pipeline steps
├── status(device_id) ───→ driver + device status
└── switch(device_id, driver_name) ───→ update device driver mapping
```

### File Layout

Drivers live in `agos-kernel/src/uri.rs`. Built-in drivers are defined at the top of the file as part of `UnifiedRoboticsInterface::new()`. The sense/actuate methods dispatch to the correct driver by name.

---

## The HardwareDriver Struct

```rust
pub struct HardwareDriver {
    pub name: String,
    pub driver_type: DriverType,    // Builtin, Runtime, Simulated
    pub capabilities: Vec<String>,  // e.g. ["sensor:temperature", "actuator:servo"]
    pub status: DriverStatus,       // Online, Offline, Error
    pub config: Value,              // driver-specific configuration
}
```

| Field | Type | Purpose |
|-------|------|---------|
| `name` | `String` | Unique driver identifier, used in REST API paths |
| `driver_type` | `DriverType` | `Builtin` (compile-time), `Runtime` (REST-registered), or `Simulated` |
| `capabilities` | `Vec<String>` | Declares what the driver can do, e.g. `sensor:temperature`, `actuator:servo` |
| `status` | `DriverStatus` | `Online`, `Offline`, or `Error` |
| `config` | `Value` | Driver-specific JSON config (pin numbers, baud rate, endpoints, etc.) |

### DeviceState

```rust
pub struct DeviceState {
    pub device_id: String,
    pub driver: String,
    pub last_value: Value,
    pub last_read: String,
    pub unit: String,
}
```

Each device is associated with exactly one driver. The `switch()` endpoint changes this mapping.

---

## The UnifiedRoboticsInterface Struct

```rust
pub struct UnifiedRoboticsInterface {
    drivers: RwLock<HashMap<String, HardwareDriver>>,
    devices: RwLock<HashMap<String, DeviceState>>,
    transforms: RwLock<Vec<DataTransform>>,
}
```

### Key Methods

| Method | Locking | Description |
|--------|---------|-------------|
| `sense(device_id, driver_name)` | RwLock read | Read sensor value via the device's driver |
| `actuate(device_id, value, driver_name)` | RwLock read + write | Write to actuator via device's driver |
| `transform(name, data)` | RwLock read | Apply a named data transform pipeline |
| `status(device_id)` | RwLock read | Return driver status + device info |
| `switch(device_id, driver_name)` | RwLock write | Re-map a device to a different driver |
| `register_driver(driver)` | RwLock write | Add a runtime driver to the registry |
| `unregister_driver(name)` | RwLock write | Remove a runtime driver from the registry |
| `get_metrics()` | RwLock read | Return per-driver call/error counters |
| `list_drivers()` | RwLock read | Return all registered drivers |

---

## Built-in Drivers

agOS ships with 4 built-in simulated drivers for development and testing:

| Driver | Name | Capabilities | Behaviour |
|--------|------|-------------|-----------|
| SimulatedTemperature | `SimulatedTemperature` | `sensor:temperature` | Returns 20–30°C sine wave |
| SimulatedServo | `SimulatedServo` | `actuator:servo` | Accepts 0–180°, echoes back |
| SimulatedDistance | `SimulatedDistance` | `sensor:distance` | Returns 0.1–10.0m random walk |
| SimulatedIMU | `SimulatedIMU` | `sensor:imu` | Returns `{roll, pitch, yaw}` tuple |

These are registered automatically in `UnifiedRoboticsInterface::new()`.

---

## Implementing a Built-in Driver

To add a new built-in driver:

### Step 1: Define the Driver

In `uri.rs`, inside `UnifiedRoboticsInterface::new()`, add a new `HardwareDriver` to the initial drivers map:

```rust
pub fn new() -> Self {
    let mut drivers = HashMap::new();

    // Existing drivers...
    drivers.insert("SimulatedTemperature".to_string(), HardwareDriver {
        name: "SimulatedTemperature".to_string(),
        driver_type: DriverType::Builtin,
        capabilities: vec!["sensor:temperature".to_string()],
        status: DriverStatus::Online,
        config: json!({"type": "simulated"}),
    });

    // New driver
    drivers.insert("MyPressureSensor".to_string(), HardwareDriver {
        name: "MyPressureSensor".to_string(),
        driver_type: DriverType::Builtin,
        capabilities: vec!["sensor:pressure".to_string()],
        status: DriverStatus::Online,
        config: json!({"type": "simulated", "min_kpa": 80.0, "max_kpa": 120.0}),
    });

    // ... init devices, transforms ...
}
```

### Step 2: Implement sense() Logic

Add a match arm in the `sense()` method to handle your driver:

```rust
pub fn sense(&self, device_id: &str, driver_name: Option<&str>) -> Result<DeviceReading, String> {
    let drivers = self.drivers.read().map_err(|e| e.to_string())?;
    let driver_name = driver_name.unwrap_or("...");
    let driver = drivers.get(driver_name).ok_or("Driver not found")?;

    let value = match driver.name.as_str() {
        "SimulatedTemperature" => json!(20.0 + (rand::random::<f64>() * 10.0)),
        "MyPressureSensor" => json!(100.0 + (rand::random::<f64>() * 20.0)),
        _ => return Err("Unsupported driver for sense".into()),
    };

    Ok(DeviceReading {
        device_id: device_id.to_string(),
        value,
        unit: "kPa".to_string(),
        driver: driver.name.clone(),
        timestamp: chrono::Utc::now().to_rfc3339(),
    })
}
```

### Step 3: Test

```bash
# Sense
curl -X POST http://127.0.0.1:7400/uri/sense \
  -H 'Content-Type: application/json' \
  -d '{"device_id": "pressure_01", "driver_name": "MyPressureSensor"}'
```

---

## Registering a Runtime Driver

Runtime drivers can be added without recompiling the kernel. The driver implementation lives in an external process or script; the kernel stores its metadata and delegates sense/actuate calls to the configured endpoint or script.

### Via REST API

```bash
curl -X POST http://127.0.0.1:7400/uri/drivers/register \
  -H 'Content-Type: application/json' \
  -d '{
    "name": "GPIO-17",
    "driver_type": "Runtime",
    "capabilities": ["sensor:gpio", "actuator:gpio"],
    "config": {
      "pin": 17,
      "mode": "input",
      "pull": "down"
    }
  }'
```

### Response

```json
{"ok": true, "name": "GPIO-17"}
```

### Unregister

```bash
curl -X POST http://127.0.0.1:7400/uri/drivers/unregister \
  -H 'Content-Type: application/json' \
  -d '{"name": "GPIO-17"}'
```

Built-in drivers cannot be unregistered:
```json
{"error": "Cannot unregister built-in driver 'SimulatedTemperature'"}
```

---

## URI Syscalls Reference

### SENSE — `sense(device_id, driver_name?) -> DeviceReading`

| Parameter | Type | Description |
|-----------|------|-------------|
| `device_id` | `&str` | Logical device identifier |
| `driver_name` | `Option<&str>` | Override the device's active driver (optional) |

Returns `DeviceReading { device_id, value, unit, driver, timestamp }`.

**Default implementation**: Dispatches to the device's current driver and returns a simulated or real reading.

**REST**:
```bash
curl -X POST http://127.0.0.1:7400/uri/sense \
  -H 'Content-Type: application/json' \
  -d '{"device_id": "temp_01"}'
```

### ACTUATE — `actuate(device_id, value, driver_name?) -> ActuateResult`

| Parameter | Type | Description |
|-----------|------|-------------|
| `device_id` | `&str` | Logical device identifier |
| `value` | `Value` | JSON value to write (number for servos, object for multi-axis) |
| `driver_name` | `Option<&str>` | Override the device's active driver (optional) |

**REST**:
```bash
curl -X POST http://127.0.0.1:7400/uri/actuate \
  -H 'Content-Type: application/json' \
  -d '{"device_id": "servo_01", "value": 90.0}'
```

### TRANSFORM — `transform(name, data) -> Vec<f64>`

| Parameter | Type | Description |
|-----------|------|-------------|
| `name` | `&str` | Name of a registered transform pipeline |
| `data` | `&[f64]` | Input data to transform |

**Supported steps**: `Normalize`, `Scale`, `Threshold`, `MovingAverage`, `Custom`.

**REST**:
```bash
curl -X POST http://127.0.0.1:7400/uri/transform \
  -H 'Content-Type: application/json' \
  -d '{"name": "normalize_temp", "data": [0.0, 23.5, 100.0]}'
```

### STATUS — `status(device_id) -> StatusInfo`

Returns driver status, device last reading, and timestamp.

**REST**:
```bash
curl -X POST http://127.0.0.1:7400/uri/status \
  -H 'Content-Type: application/json' \
  -d '{"device_id": "temp_01"}'
```

### SWITCH — `switch(device_id, driver_name)`

Re-maps a device to a different driver. Both must exist.

**REST**:
```bash
curl -X POST http://127.0.0.1:7400/uri/switch \
  -H 'Content-Type: application/json' \
  -d '{"device_id": "temp_01", "driver_name": "SimulatedTemperature"}'
```

---

## End-to-End Walkthrough: Adding a GPIO Driver

This walkthrough adds a runtime driver for a physical GPIO pin via sysfs.

### Step 1: Register the Driver

```bash
curl -X POST http://127.0.0.1:7400/uri/drivers/register \
  -H 'Content-Type: application/json' \
  -d '{
    "name": "SysfsGPIO-17",
    "driver_type": "Runtime",
    "capabilities": ["sensor:gpio", "actuator:gpio"],
    "config": {"pin": 17, "direction": "in"}
  }'
```

### Step 2: Create a Device

Map a device ID to the driver via the switch endpoint:
```bash
curl -X POST http://127.0.0.1:7400/uri/switch \
  -H 'Content-Type: application/json' \
  -d '{"device_id": "button_01", "driver_name": "SysfsGPIO-17"}'
```

### Step 3: Sense

```bash
curl -X POST http://127.0.0.1:7400/uri/sense \
  -H 'Content-Type: application/json' \
  -d '{"device_id": "button_01"}'
```

### Step 4: Implement Actual I/O (External)

For a runtime driver, the kernel stores the metadata, but the actual I/O logic needs to be implemented as an external process that the kernel can call, OR via a WASM sandbox tool that performs the GPIO read/write.

---

## Best Practices

1. **Capability naming** — Use `domain:action` format (e.g., `sensor:temperature`, `actuator:servo`). This matches the Ring 0 permission model.
2. **Simulated defaults** — Always provide a simulated fallback driver for development and testing.
3. **Unit annotations** — Every device should declare its units (celsius, degrees, meters, kPa) so agents can interpret values correctly.
4. **Error handling** — Never panic in a driver method. Return `Err(String)` with a descriptive message.
5. **Runtime driver isolation** — Runtime drivers are metadata-only. Actual hardware I/O should be implemented as WASM sandbox tools or external services for security.
6. **Device naming** — Use descriptive device IDs that include the physical location or function (e.g., `joint_1_theta`, `oven_temp`, `front_left_wheel`).
7. **Transform pipelines** — Keep transforms stateless and pure. Stateful operations (like moving averages) should store state in the transform name or device config.

---

## Troubleshooting

### Driver not found

Check that the driver name exactly matches what was registered:
```bash
curl http://127.0.0.1:7400/uri/drivers
```

### sense() returns error

- Verify the device has been mapped to a driver via `switch()`.
- Check the driver status is `Online`.

### Cannot unregister driver

Built-in drivers cannot be unregistered:
```bash
# Verify driver type
curl http://127.0.0.1:7400/uri/drivers/SimulatedTemperature
```

### No transforms available

Register a transform via the hardware DB or add one to the initial set in `UnifiedRoboticsInterface::new()`.

### Metrics not appearing

Ensure the driver's call/error counters are being incremented in `sense()` and `actuate()`:
```bash
curl http://127.0.0.1:7400/uri/metrics
```

---

## Reference: Complete Driver Checklist

- [ ] `HardwareDriver` struct defined with name, type, capabilities, status, config
- [ ] Driver registered in the drivers map (builtin) or via REST (runtime)
- [ ] Device mapped to driver via `switch()` or initial device list
- [ ] `sense()` handles the driver name with appropriate I/O logic
- [ ] `actuate()` handles the driver name with appropriate I/O logic
- [ ] Driver appears in `list_drivers()` response
- [ ] Driver metrics appear in `get_metrics()` response
- [ ] Kernel compiles with `cargo build --features uri`
- [ ] Sense works via REST
- [ ] Actuate works via REST
- [ ] Switch works via REST
