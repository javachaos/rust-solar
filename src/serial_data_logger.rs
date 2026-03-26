//! Controller transport boundary and serial-backed implementation.
//!
//! The app talks to MPPT hardware through [`MpptController`]. That keeps the UI and
//! persistence layers independent from whether telemetry comes from:
//!
//! - a real serial device
//! - the in-process simulator transport
//! - the Unix PTY-backed simulator harness

use std::{
    fmt,
    io::{self, Read, Write},
    str::FromStr,
    time::Duration,
};

use log::info;
use serialport::SerialPort;

#[cfg(unix)]
use serialport::TTYPort;

use crate::datapoint::{DataPoint, DataPointParseError};

/// Commands the UI can send to a controller implementation.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub(crate) enum ControllerCommand {
    SetLoad(bool),
}

/// Abstraction over sources that can yield MPPT datapoints and accept load commands.
pub(crate) trait MpptController: Send {
    /// Performs any startup work required before normal reads begin.
    fn prime(&mut self) -> Result<(), ControllerError>;
    /// Reads the next telemetry datapoint from the controller.
    fn read_datapoint(&mut self) -> Result<DataPoint, ControllerError>;
    /// Changes the controller's load output state.
    fn set_load_enabled(&mut self, enabled: bool) -> Result<(), ControllerError>;
}

/// Errors that can happen while talking to a controller.
#[derive(Debug)]
pub(crate) enum ControllerError {
    Io(io::Error),
    Serial(serialport::Error),
    Parse(DataPointParseError),
}

impl fmt::Display for ControllerError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io(err) => write!(f, "controller I/O error: {err}"),
            Self::Serial(err) => write!(f, "serial controller error: {err}"),
            Self::Parse(err) => write!(f, "invalid controller payload: {err}"),
        }
    }
}

impl std::error::Error for ControllerError {}

impl From<io::Error> for ControllerError {
    fn from(value: io::Error) -> Self {
        Self::Io(value)
    }
}

impl From<serialport::Error> for ControllerError {
    fn from(value: serialport::Error) -> Self {
        Self::Serial(value)
    }
}

impl From<DataPointParseError> for ControllerError {
    fn from(value: DataPointParseError) -> Self {
        Self::Parse(value)
    }
}

/// Adapter that reads newline-delimited controller frames from a transport and parses them
/// into [`DataPoint`] values.
pub(crate) struct SerialDatalogger<T> {
    port: T,
}

#[cfg_attr(test, allow(dead_code))]
impl SerialDatalogger<Box<dyn SerialPort>> {
    const BAUD_RATE: u32 = 57_600;
    const SERIAL_TIMEOUT_MS: u64 = 2_000;

    /// Returns the serial ports currently reported by the host OS.
    pub(crate) fn get_comms() -> Result<Vec<String>, ControllerError> {
        Ok(serialport::available_ports()?
            .into_iter()
            .map(|port| port.port_name)
            .collect())
    }

    /// Opens a real serial device using the controller's configured baud rate.
    pub(crate) fn connect(port_name: &str) -> Result<Self, ControllerError> {
        serialport::new(port_name, Self::BAUD_RATE)
            .timeout(Duration::from_millis(Self::SERIAL_TIMEOUT_MS))
            .open()
            .map(Self::from_port)
            .map_err(ControllerError::from)
    }

    #[cfg(unix)]
    /// Opens a PTY-backed device on Unix systems.
    ///
    /// PTYs are used by the end-to-end simulator harness, so this path intentionally avoids
    /// the standard baud-rate setup used for real hardware.
    pub(crate) fn connect_pty(port_name: &str) -> Result<Self, ControllerError> {
        TTYPort::open(
            &serialport::new(port_name, 0).timeout(Duration::from_millis(Self::SERIAL_TIMEOUT_MS)),
        )
        .map(|port| Self::from_port(Box::new(port) as Box<dyn SerialPort>))
        .map_err(ControllerError::from)
    }
}

impl<T> SerialDatalogger<T> {
    /// Wraps an arbitrary transport that already implements the controller byte protocol.
    pub(crate) fn from_port(port: T) -> Self {
        Self { port }
    }
}

impl<T> SerialDatalogger<T>
where
    T: Read + Write + Send,
{
    /// Reads a single newline-delimited controller frame from the underlying transport.
    fn read_serial_datapoint(&mut self) -> Result<String, ControllerError> {
        let mut buffer = Vec::new();
        let mut byte = [0u8; 1];

        loop {
            let bytes_read = self.port.read(&mut byte)?;
            if bytes_read == 0 {
                break;
            }

            buffer.push(byte[0]);

            if byte[0] == b'\n' {
                break;
            }
        }

        if buffer.is_empty() {
            return Err(
                io::Error::new(io::ErrorKind::UnexpectedEof, "no serial data received").into(),
            );
        }

        Ok(String::from_utf8_lossy(&buffer)
            .trim_end_matches(&['\r', '\n'][..])
            .to_string())
    }

    /// Writes a raw controller command and flushes it immediately.
    fn write_command(&mut self, data: &str) -> Result<usize, ControllerError> {
        self.port.write_all(data.as_bytes())?;
        self.port.flush()?;
        Ok(data.len())
    }
}

impl<T> MpptController for SerialDatalogger<T>
where
    T: Read + Write + Send,
{
    fn prime(&mut self) -> Result<(), ControllerError> {
        let _ = self.read_serial_datapoint()?;
        Ok(())
    }

    fn read_datapoint(&mut self) -> Result<DataPoint, ControllerError> {
        let payload = self.read_serial_datapoint()?;
        Ok(DataPoint::from_str(&payload)?)
    }

    fn set_load_enabled(&mut self, enabled: bool) -> Result<(), ControllerError> {
        let command = if enabled { "LON\n" } else { "LOFF\n" };
        let bytes_written = self.write_command(command)?;
        info!("Wrote {bytes_written} bytes over serial.");
        Ok(())
    }
}

#[cfg(test)]
mod tests {
    use std::io::{self, Cursor, Read, Result as IoResult, Write};

    #[cfg(unix)]
    use std::sync::Mutex;

    #[cfg(unix)]
    use std::time::Duration;

    use super::{ControllerError, MpptController, SerialDatalogger};
    use crate::datapoint::DataPointParseError;

    #[cfg(unix)]
    use crate::pty_controller_harness::PtyControllerHarness;

    #[cfg(unix)]
    static SERIAL_TEST_LOCK: Mutex<()> = Mutex::new(());

    #[derive(Debug)]
    struct MockTransport {
        read_buffer: Cursor<Vec<u8>>,
        writes: Vec<u8>,
        flush_count: usize,
    }

    impl MockTransport {
        fn new(input: &str) -> Self {
            Self {
                read_buffer: Cursor::new(input.as_bytes().to_vec()),
                writes: Vec::new(),
                flush_count: 0,
            }
        }
    }

    impl Read for MockTransport {
        fn read(&mut self, buf: &mut [u8]) -> IoResult<usize> {
            self.read_buffer.read(buf)
        }
    }

    impl Write for MockTransport {
        fn write(&mut self, buf: &[u8]) -> IoResult<usize> {
            self.writes.extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> IoResult<()> {
            self.flush_count += 1;
            Ok(())
        }
    }

    #[test]
    fn prime_consumes_the_first_payload() {
        let transport = MockTransport::new("ignore-me\n12.5:8.4:4.2:10.8:14.9:1:1:22:3.8:0\n");
        let mut logger = SerialDatalogger::from_port(transport);

        logger.prime().expect("prime should succeed");
        let datapoint = logger.read_datapoint().expect("datapoint should parse");

        assert_eq!(datapoint.get_charge_current(), 3.8);
    }

    #[test]
    fn read_datapoint_trims_newlines() {
        let transport = MockTransport::new("12.5:8.4:4.2:10.8:14.9:1:1:22:3.8:1\r\n");
        let mut logger = SerialDatalogger::from_port(transport);

        let datapoint = logger.read_datapoint().expect("datapoint should parse");

        assert!(datapoint.is_load_enabled());
        assert_eq!(datapoint.get_battery_voltage(), 12.5);
    }

    #[test]
    fn read_datapoint_returns_parse_errors_for_invalid_payloads() {
        let transport = MockTransport::new("invalid-payload\n");
        let mut logger = SerialDatalogger::from_port(transport);

        let err = logger
            .read_datapoint()
            .expect_err("invalid payload should return an error");

        assert!(matches!(err, ControllerError::Parse(_)));
    }

    #[test]
    fn read_datapoint_returns_io_error_when_no_bytes_are_available() {
        let transport = MockTransport::new("");
        let mut logger = SerialDatalogger::from_port(transport);

        let err = logger
            .read_datapoint()
            .expect_err("empty input should return an error");

        assert!(matches!(err, ControllerError::Io(_)));
    }

    #[test]
    fn set_load_enabled_writes_expected_commands() {
        let transport = MockTransport::new("");
        let mut logger = SerialDatalogger::from_port(transport);

        logger
            .set_load_enabled(true)
            .expect("load on command should succeed");
        assert_eq!(logger.port.writes, b"LON\n");
        assert_eq!(logger.port.flush_count, 1);

        logger.port.writes.clear();
        logger
            .set_load_enabled(false)
            .expect("load off command should succeed");

        assert_eq!(logger.port.writes, b"LOFF\n");
        assert_eq!(logger.port.flush_count, 2);
    }

    #[test]
    fn controller_error_display_messages_are_human_readable() {
        let io_error = ControllerError::from(io::Error::new(io::ErrorKind::BrokenPipe, "lost"));
        let serial_error = ControllerError::from(serialport::Error::new(
            serialport::ErrorKind::NoDevice,
            "missing port",
        ));
        let parse_error = ControllerError::from(DataPointParseError::InvalidFieldCount {
            expected: 10,
            actual: 2,
        });

        assert!(io_error.to_string().contains("controller I/O error"));
        assert!(serial_error.to_string().contains("serial controller error"));
        assert!(parse_error
            .to_string()
            .contains("invalid controller payload"));
    }

    #[test]
    fn get_comms_returns_without_error() {
        let ports = SerialDatalogger::get_comms().expect("port enumeration should succeed");

        assert!(
            ports.iter().all(|port| !port.is_empty()),
            "enumerated port names should be non-empty"
        );
    }

    #[test]
    fn connect_returns_serial_errors_for_missing_ports() {
        assert!(matches!(
            SerialDatalogger::connect("definitely-not-a-real-serial-port"),
            Err(ControllerError::Serial(_))
        ));
    }

    #[cfg(target_os = "linux")]
    #[test]
    fn connect_pty_and_plain_connect_both_open_linux_ptys() {
        let _guard = SERIAL_TEST_LOCK
            .lock()
            .expect("serial test lock should be available");
        let harness = PtyControllerHarness::spawn_with_interval(Duration::from_millis(25))
            .expect("harness should spawn");

        let mut tty_logger = SerialDatalogger::connect_pty(harness.slave_path())
            .expect("pty connection should open");
        tty_logger.prime().expect("pty logger should prime");
        let tty_datapoint = tty_logger
            .read_datapoint()
            .expect("pty logger should read datapoints");
        assert!(!tty_datapoint.is_load_enabled());
        drop(tty_logger);

        let mut serial_logger =
            SerialDatalogger::connect(harness.slave_path()).expect("plain serial open should work");
        serial_logger
            .prime()
            .expect("plain serial logger should prime on linux PTYs");
        let serial_datapoint = serial_logger
            .read_datapoint()
            .expect("plain serial logger should read datapoints");
        assert!(!serial_datapoint.is_load_enabled());
    }

    #[cfg(target_os = "macos")]
    #[test]
    fn connect_pty_opens_the_harness_and_plain_connect_rejects_macos_ptys() {
        let _guard = SERIAL_TEST_LOCK
            .lock()
            .expect("serial test lock should be available");
        let harness = PtyControllerHarness::spawn_with_interval(Duration::from_millis(25))
            .expect("harness should spawn");

        let mut tty_logger = SerialDatalogger::connect_pty(harness.slave_path())
            .expect("pty connection should open");
        tty_logger.prime().expect("pty logger should prime");
        let tty_datapoint = tty_logger
            .read_datapoint()
            .expect("pty logger should read datapoints");
        assert!(!tty_datapoint.is_load_enabled());
        drop(tty_logger);

        assert!(matches!(
            SerialDatalogger::connect(harness.slave_path()),
            Err(ControllerError::Serial(_))
        ));
    }

    #[cfg(target_os = "windows")]
    #[test]
    fn windows_uses_plain_serial_connect_and_reports_missing_ports() {
        assert!(matches!(
            SerialDatalogger::connect("definitely-not-a-real-serial-port"),
            Err(ControllerError::Serial(_))
        ));
    }
}
