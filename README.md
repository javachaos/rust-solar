# rust-solar
A simple solar tracer app re-written in rust.

# Running

```
cargo run
```

# Architecture

The codebase is organized around a few small responsibilities:

- `src/app.rs`: terminal UI, thread orchestration, port selection, and dashboard rendering
- `src/serial_data_logger.rs`: transport abstraction plus the real serial implementation
- `src/controller_simulator.rs`: in-process MPPT simulator and serial-style byte wrapper
- `src/pty_controller_harness.rs`: Unix PTY wrapper for end-to-end serial-path testing
- `src/datapoint.rs`: parser and model for controller telemetry frames
- `src/database.rs`: buffered SQLite persistence

# Controller Frame Format

Incoming controller frames are expected to be colon-delimited with 10 numeric fields:

```text
battery_voltage:pv_voltage:load_current:over_discharge:battery_max:battery_full:charging:battery_temp:charge_current:load_onoff
```

Example:

```text
12.50:18.20:4.10:10.70:14.80:1:1:24.00:5.60:1
```

# PTY Simulator

For an end-to-end pseudo-serial controller on Unix-like systems:

```bash
cargo run --bin pty_controller_sim
```

That prints a PTY device path. In a second terminal, run the app and inject that path into the
port list:

```bash
SOLAR_EXTRA_PORTS=pty:/path/from/simulator cargo run
```

# Testing

Run the test suite with:

```bash
cargo test
```

The project includes:

- parser and formatting tests for controller datapoints
- database persistence and buffering tests
- UI rendering and input handling tests
- simulator and serial transport tests
- PTY-backed end-to-end tests on Unix-like systems

# How to use.
 - Select a COM port from the initial list.
 - Once the app is running, you can use the mouse to click LOAD on or off.
 - The display will update once per second.
 - To quit press q.
# Screenshot
![.](https://github.com/javachaos/rust-solar/blob/main/assets/screenshot.png)
