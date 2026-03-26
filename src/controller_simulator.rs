//! In-process MPPT controller simulation used by tests and the UI's "no hardware" mode.
//!
//! The simulator is intentionally simple but realistic enough to exercise the app's core
//! assumptions:
//!
//! - telemetry is emitted as the same colon-delimited frame format as the real controller
//! - load commands use the same `LON` / `LOFF` text protocol
//! - a byte-stream wrapper is available so the serial parser is tested instead of bypassed

use std::{
    collections::VecDeque,
    io::{self, Read, Write},
    str,
};

use crate::{
    datapoint::DataPoint,
    serial_data_logger::{ControllerCommand, ControllerError, MpptController},
};

/// Display name shown in the port-selection UI for the in-process simulator path.
pub(crate) const SIMULATOR_PORT_NAME: &str = "Simulator (no hardware)";

/// Deterministic MPPT simulator that emits controller frames and accepts load commands.
#[derive(Debug, Clone)]
pub(crate) struct MpptControllerSimulator {
    tick: u64,
    load_enabled: bool,
}

impl Default for MpptControllerSimulator {
    fn default() -> Self {
        Self {
            tick: 0,
            load_enabled: false,
        }
    }
}

impl MpptControllerSimulator {
    /// Produces one controller frame using the app's 10-field colon-delimited format.
    pub(crate) fn next_frame(&mut self) -> String {
        self.tick = self.tick.wrapping_add(1);

        let battery_voltage = self.sample_range(11.5, 14.8, 32);
        let pv_voltage = self.sample_range(0.5, 20.0, 24);
        let load_current = if self.load_enabled {
            self.sample_range(0.8, 12.5, 20)
        } else {
            0.0
        };
        let over_discharge = self.sample_range(10.4, 11.2, 16);
        let battery_max = 14.8;
        let battery_full = if battery_voltage >= 14.1 { 1.0 } else { 0.0 };
        let charging = if pv_voltage > battery_voltage + 0.5 {
            1.0
        } else {
            0.0
        };
        let battery_temp = self.sample_range(-5.0, 30.0, 18);
        let charge_current = if charging > 0.0 {
            self.sample_range(0.0, 15.0, 14)
        } else {
            0.0
        };
        let load_onoff = if self.load_enabled { 1.0 } else { 0.0 };

        [
            battery_voltage,
            pv_voltage,
            load_current,
            over_discharge,
            battery_max,
            battery_full,
            charging,
            battery_temp,
            charge_current,
            load_onoff,
        ]
        .into_iter()
        .map(format_value)
        .collect::<Vec<_>>()
        .join(":")
    }

    /// Produces one controller frame terminated with `\n`, matching the serial transport.
    pub(crate) fn next_frame_line(&mut self) -> String {
        let mut frame = self.next_frame();
        frame.push('\n');
        frame
    }

    /// Applies a parsed controller command directly to the simulated state.
    pub(crate) fn apply_command(&mut self, command: ControllerCommand) {
        match command {
            ControllerCommand::SetLoad(enabled) => self.load_enabled = enabled,
        }
    }

    /// Applies a raw textual command such as `LON` or `LOFF`.
    pub(crate) fn apply_command_text(&mut self, command: &str) -> io::Result<()> {
        match command.trim() {
            "LON" => self.apply_command(ControllerCommand::SetLoad(true)),
            "LOFF" => self.apply_command(ControllerCommand::SetLoad(false)),
            "" => {}
            other => {
                return Err(io::Error::new(
                    io::ErrorKind::InvalidInput,
                    format!("unknown simulator command: {other}"),
                ));
            }
        }

        Ok(())
    }

    fn sample_range(&self, min: f64, max: f64, period: u64) -> f64 {
        let phase = (self.tick % period) as f64 / period as f64;
        let value = min + ((max - min) * phase);
        (value * 100.0).round() / 100.0
    }
}

/// Thin controller implementation used by tests that do not need byte-level transport behavior.
#[cfg_attr(not(test), allow(dead_code))]
pub(crate) struct SimulatedController {
    simulator: MpptControllerSimulator,
}

impl Default for SimulatedController {
    fn default() -> Self {
        Self {
            simulator: MpptControllerSimulator::default(),
        }
    }
}

impl MpptController for SimulatedController {
    fn prime(&mut self) -> Result<(), ControllerError> {
        Ok(())
    }

    fn read_datapoint(&mut self) -> Result<DataPoint, ControllerError> {
        Ok(self.simulator.next_frame().parse()?)
    }

    fn set_load_enabled(&mut self, enabled: bool) -> Result<(), ControllerError> {
        self.simulator
            .apply_command(ControllerCommand::SetLoad(enabled));
        Ok(())
    }
}

/// `Read`/`Write` wrapper around [`MpptControllerSimulator`] that behaves like a serial link.
///
/// Reads yield newline-delimited frames. Writes buffer incoming bytes until a newline is seen,
/// then apply the parsed command to the simulator.
#[derive(Debug, Default)]
pub(crate) struct SimulatedSerialTransport {
    simulator: MpptControllerSimulator,
    pending_frame: VecDeque<u8>,
    pending_command: Vec<u8>,
}

impl SimulatedSerialTransport {
    fn ensure_frame_available(&mut self) {
        if self.pending_frame.is_empty() {
            self.pending_frame
                .extend(self.simulator.next_frame_line().into_bytes());
        }
    }

    fn handle_command(&mut self, command: &str) -> io::Result<()> {
        self.simulator.apply_command_text(command)?;

        // Discard any stale frame so the next read reflects the latest controller state.
        self.pending_frame.clear();
        Ok(())
    }
}

impl Read for SimulatedSerialTransport {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        if buf.is_empty() {
            return Ok(0);
        }

        self.ensure_frame_available();

        let mut bytes_read = 0;
        while bytes_read < buf.len() {
            let Some(byte) = self.pending_frame.pop_front() else {
                break;
            };

            buf[bytes_read] = byte;
            bytes_read += 1;
        }

        Ok(bytes_read)
    }
}

impl Write for SimulatedSerialTransport {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.pending_command.extend_from_slice(buf);

        while let Some(newline_index) = self.pending_command.iter().position(|byte| *byte == b'\n')
        {
            let command_bytes = self
                .pending_command
                .drain(..=newline_index)
                .collect::<Vec<_>>();
            let command = str::from_utf8(&command_bytes).map_err(|err| {
                io::Error::new(
                    io::ErrorKind::InvalidData,
                    format!("simulator command was not valid UTF-8: {err}"),
                )
            })?;
            self.handle_command(command)?;
        }

        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn format_value(value: f64) -> String {
    if value.fract().abs() < f64::EPSILON {
        format!("{value:.0}")
    } else {
        format!("{value:.2}")
    }
}

#[cfg(test)]
mod tests {
    use std::io::{ErrorKind, Read, Write};

    use super::{MpptControllerSimulator, SimulatedController, SimulatedSerialTransport};
    use crate::serial_data_logger::{MpptController, SerialDatalogger};

    #[test]
    fn simulator_frames_are_parseable() {
        let mut simulator = MpptControllerSimulator::default();
        let frame = simulator.next_frame();
        let datapoint = frame.parse::<crate::datapoint::DataPoint>();

        assert!(datapoint.is_ok(), "simulator should emit parseable frames");
    }

    #[test]
    fn simulator_reflects_load_state_changes() {
        let mut controller = SimulatedController::default();

        let before_toggle = controller
            .read_datapoint()
            .expect("simulator datapoint should be readable");
        assert!(!before_toggle.is_load_enabled());

        controller
            .set_load_enabled(true)
            .expect("simulator toggle should succeed");
        let after_toggle = controller
            .read_datapoint()
            .expect("simulator datapoint should be readable");

        assert!(after_toggle.is_load_enabled());
    }

    #[test]
    fn text_commands_toggle_simulator_state() {
        let mut simulator = MpptControllerSimulator::default();

        simulator
            .apply_command_text("LON")
            .expect("text command should succeed");
        assert!(simulator.next_frame().ends_with(":1"));

        simulator
            .apply_command_text("LOFF")
            .expect("text command should succeed");
        assert!(simulator.next_frame().ends_with(":0"));
    }

    #[test]
    fn empty_text_commands_are_ignored() {
        let mut simulator = MpptControllerSimulator::default();

        simulator
            .apply_command_text("")
            .expect("empty commands should be ignored");

        assert!(simulator.next_frame().ends_with(":0"));
    }

    #[test]
    fn invalid_text_commands_return_an_error() {
        let mut simulator = MpptControllerSimulator::default();
        let err = simulator
            .apply_command_text("UNKNOWN")
            .expect_err("unknown commands should fail");

        assert_eq!(err.kind(), ErrorKind::InvalidInput);
    }

    #[test]
    fn simulator_emits_charging_frames_with_nonzero_charge_current() {
        let mut simulator = MpptControllerSimulator::default();

        let datapoint = (0..64)
            .map(|_| {
                simulator
                    .next_frame()
                    .parse::<crate::datapoint::DataPoint>()
                    .expect("simulator frames should parse")
            })
            .find(|datapoint| datapoint.is_charging())
            .expect("simulator should eventually emit a charging frame");

        assert!(datapoint.get_charge_current() > 0.0);
    }

    #[test]
    fn simulated_controller_prime_is_a_noop() {
        let mut controller = SimulatedController::default();

        controller.prime().expect("prime should succeed");
    }

    #[test]
    fn simulated_serial_transport_handles_empty_reads_and_large_buffers() {
        let mut transport = SimulatedSerialTransport::default();
        let mut empty = [];
        assert_eq!(
            transport
                .read(&mut empty)
                .expect("empty reads should succeed"),
            0
        );

        let mut large_buffer = [0u8; 256];
        let bytes_read = transport
            .read(&mut large_buffer)
            .expect("reading a full frame should succeed");

        assert!(bytes_read > 0);
        assert!(large_buffer[..bytes_read].contains(&b'\n'));
    }

    #[test]
    fn simulated_serial_transport_rejects_invalid_utf8_commands() {
        let mut transport = SimulatedSerialTransport::default();
        let err = transport
            .write(&[0xFF, b'\n'])
            .expect_err("invalid UTF-8 commands should fail");

        assert_eq!(err.kind(), ErrorKind::InvalidData);
    }

    #[test]
    fn serial_transport_wrapper_exercises_end_to_end_controller_flow() {
        let transport = SimulatedSerialTransport::default();
        let mut controller = SerialDatalogger::from_port(transport);

        let before_toggle = controller
            .read_datapoint()
            .expect("wrapped simulator datapoint should be readable");
        assert!(!before_toggle.is_load_enabled());

        controller
            .set_load_enabled(true)
            .expect("wrapped simulator toggle should succeed");
        let after_toggle = controller
            .read_datapoint()
            .expect("wrapped simulator datapoint should be readable");

        assert!(after_toggle.is_load_enabled());
        assert!(after_toggle.get_load_current() > 0.0);
    }
}
