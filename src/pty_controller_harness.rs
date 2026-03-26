//! Unix-only PTY wrapper around the in-process controller simulator.
//!
//! This module exists for the "last mile" tests: instead of talking to the simulator
//! through an in-memory transport, the app can open an actual PTY slave path and exercise
//! the same serial open, read, and write code used for real devices.

#[cfg(unix)]
use std::{
    io::{self, Read, Write},
    sync::mpsc::{self, Receiver},
    thread::{self, JoinHandle},
    time::{Duration, Instant},
};

#[cfg(unix)]
use serialport::{SerialPort, TTYPort};

#[cfg(unix)]
use crate::controller_simulator::MpptControllerSimulator;

#[cfg(unix)]
const DEFAULT_FRAME_INTERVAL: Duration = Duration::from_secs(1);

#[cfg(unix)]
const COMMAND_POLL_INTERVAL: Duration = Duration::from_millis(20);

/// Prefix used by the UI to recognize PTY-backed simulator ports injected through
/// `SOLAR_EXTRA_PORTS`.
#[cfg(unix)]
pub const PTY_PORT_PREFIX: &str = "pty:";

/// Keeps a PTY-backed simulator thread alive and exposes the slave path to callers.
#[cfg(unix)]
pub struct PtyControllerHarness {
    slave_path: String,
    _slave_guard: TTYPort,
    shutdown_tx: mpsc::Sender<()>,
    thread_handle: Option<JoinHandle<()>>,
}

#[cfg(unix)]
impl PtyControllerHarness {
    /// Spawns a PTY-backed simulator using the default frame interval.
    pub fn spawn() -> io::Result<Self> {
        Self::spawn_with_interval(DEFAULT_FRAME_INTERVAL)
    }

    /// Spawns a PTY-backed simulator using an explicit telemetry interval.
    pub fn spawn_with_interval(frame_interval: Duration) -> io::Result<Self> {
        let (mut master, slave) = TTYPort::pair().map_err(to_io_error)?;
        master
            .set_timeout(COMMAND_POLL_INTERVAL)
            .map_err(to_io_error)?;

        let slave_path = slave
            .name()
            .ok_or_else(|| io::Error::new(io::ErrorKind::NotFound, "pty slave path not found"))?;

        let (shutdown_tx, shutdown_rx) = mpsc::channel();
        let thread_handle = thread::Builder::new()
            .name("pty-controller-simulator".into())
            .spawn(move || run_pty_simulator_thread(master, frame_interval, shutdown_rx))?;

        Ok(Self {
            slave_path,
            _slave_guard: slave,
            shutdown_tx,
            thread_handle: Some(thread_handle),
        })
    }

    /// Returns the PTY slave path that the application should open.
    pub fn slave_path(&self) -> &str {
        &self.slave_path
    }
}

#[cfg(unix)]
impl Drop for PtyControllerHarness {
    fn drop(&mut self) {
        let _ = self.shutdown_tx.send(());
        if let Some(thread_handle) = self.thread_handle.take() {
            let _ = thread_handle.join();
        }
    }
}

#[cfg(unix)]
/// Drives the simulator thread and reports any terminal error to stderr.
fn run_pty_simulator_thread<T>(master: T, frame_interval: Duration, shutdown_rx: Receiver<()>)
where
    T: Read + Write,
{
    if let Err(err) = run_pty_simulator(master, frame_interval, shutdown_rx) {
        eprintln!("pty controller simulator stopped: {err}");
    }
}

#[cfg(unix)]
/// Emits controller frames to the PTY master and applies commands read back from it.
fn run_pty_simulator<T>(
    mut master: T,
    frame_interval: Duration,
    shutdown_rx: Receiver<()>,
) -> io::Result<()>
where
    T: Read + Write,
{
    let mut simulator = MpptControllerSimulator::default();
    let mut pending_command = Vec::new();

    loop {
        if shutdown_requested(&shutdown_rx) {
            return Ok(());
        }

        master.write_all(simulator.next_frame_line().as_bytes())?;
        master.flush()?;

        let deadline = Instant::now() + frame_interval;
        while Instant::now() < deadline {
            if shutdown_requested(&shutdown_rx) {
                return Ok(());
            }

            read_pending_commands(&mut master, &mut simulator, &mut pending_command)?;
        }
    }
}

#[cfg(unix)]
/// Reads pending command bytes from the PTY master and applies complete newline-delimited
/// commands to the simulator.
fn read_pending_commands<T>(
    master: &mut T,
    simulator: &mut MpptControllerSimulator,
    pending_command: &mut Vec<u8>,
) -> io::Result<()>
where
    T: Read,
{
    let mut buffer = [0u8; 128];
    match master.read(&mut buffer) {
        Ok(0) => Ok(()),
        Ok(bytes_read) => {
            pending_command.extend_from_slice(&buffer[..bytes_read]);

            while let Some(newline_index) = pending_command.iter().position(|byte| *byte == b'\n') {
                let command_bytes = pending_command.drain(..=newline_index).collect::<Vec<_>>();
                let command = std::str::from_utf8(&command_bytes).map_err(|err| {
                    io::Error::new(
                        io::ErrorKind::InvalidData,
                        format!("pty simulator command was not valid UTF-8: {err}"),
                    )
                })?;
                simulator.apply_command_text(command)?;
            }

            Ok(())
        }
        Err(err) if err.kind() == io::ErrorKind::TimedOut => Ok(()),
        Err(err) => Err(err),
    }
}

#[cfg(unix)]
fn shutdown_requested(shutdown_rx: &Receiver<()>) -> bool {
    shutdown_rx.try_recv().is_ok()
}

#[cfg(unix)]
/// Converts `serialport` errors into `io::Error` values so thread helpers can share a common
/// error type.
fn to_io_error(err: serialport::Error) -> io::Error {
    io::Error::new(io::ErrorKind::Other, err.to_string())
}

#[cfg(all(test, unix))]
mod tests {
    use std::{
        collections::VecDeque,
        io::{self, Read, Write},
        sync::{mpsc, Mutex},
        time::Duration,
    };

    use serialport::{ErrorKind, SerialPort, TTYPort};

    use super::{
        read_pending_commands, run_pty_simulator, run_pty_simulator_thread, to_io_error,
        MpptControllerSimulator, PtyControllerHarness,
    };
    use crate::serial_data_logger::{MpptController, SerialDatalogger};

    static PTY_TEST_LOCK: Mutex<()> = Mutex::new(());

    #[derive(Debug)]
    enum ReadStep {
        Bytes(Vec<u8>),
        Error(io::ErrorKind),
    }

    #[derive(Debug, Default)]
    struct MockPtyTransport {
        read_steps: VecDeque<ReadStep>,
        writes: Vec<u8>,
        fail_write: Option<io::ErrorKind>,
    }

    impl MockPtyTransport {
        fn with_read_steps(read_steps: impl IntoIterator<Item = ReadStep>) -> Self {
            Self {
                read_steps: read_steps.into_iter().collect(),
                writes: Vec::new(),
                fail_write: None,
            }
        }

        fn failing_on_write(kind: io::ErrorKind) -> Self {
            Self {
                read_steps: VecDeque::new(),
                writes: Vec::new(),
                fail_write: Some(kind),
            }
        }
    }

    impl Read for MockPtyTransport {
        fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
            match self.read_steps.pop_front() {
                Some(ReadStep::Bytes(bytes)) => {
                    let bytes_to_copy = bytes.len().min(buf.len());
                    buf[..bytes_to_copy].copy_from_slice(&bytes[..bytes_to_copy]);
                    Ok(bytes_to_copy)
                }
                Some(ReadStep::Error(kind)) => Err(io::Error::new(kind, "mock read error")),
                None => Ok(0),
            }
        }
    }

    impl Write for MockPtyTransport {
        fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
            if let Some(kind) = self.fail_write.take() {
                return Err(io::Error::new(kind, "mock write error"));
            }

            self.writes.extend_from_slice(buf);
            Ok(buf.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn harness_spawn_uses_default_interval() {
        let _guard = PTY_TEST_LOCK
            .lock()
            .expect("pty test lock should be available");
        let harness = PtyControllerHarness::spawn().expect("default harness should spawn");

        assert!(!harness.slave_path().is_empty());
    }

    #[test]
    fn harness_exposes_a_real_serial_device_path() {
        let _guard = PTY_TEST_LOCK
            .lock()
            .expect("pty test lock should be available");
        let harness = PtyControllerHarness::spawn_with_interval(Duration::from_millis(25))
            .expect("harness should spawn");
        let mut controller = SerialDatalogger::connect_pty(harness.slave_path())
            .expect("serial datalogger should connect");

        controller.prime().expect("controller should prime");
        let before_toggle = controller
            .read_datapoint()
            .expect("controller should read a datapoint");
        assert!(!before_toggle.is_load_enabled());

        controller
            .set_load_enabled(true)
            .expect("load toggle should succeed");

        let after_toggle = (0..6)
            .map(|_| {
                controller
                    .read_datapoint()
                    .expect("controller should read a datapoint")
            })
            .find(|datapoint| datapoint.is_load_enabled())
            .expect("controller should eventually reflect the load state change");

        assert!(after_toggle.is_load_enabled());
    }

    #[test]
    fn simulator_loop_exits_immediately_when_shutdown_is_requested() {
        let transport = MockPtyTransport::default();
        let (shutdown_tx, shutdown_rx) = mpsc::channel();
        shutdown_tx
            .send(())
            .expect("shutdown signal should be sent successfully");

        run_pty_simulator(transport, Duration::from_millis(10), shutdown_rx)
            .expect("pre-shutdown simulator loop should exit cleanly");
    }

    #[test]
    fn read_pending_commands_ignores_zero_length_reads() {
        let mut transport = MockPtyTransport::default();
        let mut simulator = MpptControllerSimulator::default();
        let mut pending_command = Vec::new();

        read_pending_commands(&mut transport, &mut simulator, &mut pending_command)
            .expect("zero-length reads should be ignored");

        assert!(pending_command.is_empty());
        assert!(simulator.next_frame().ends_with(":0"));
    }

    #[test]
    fn read_pending_commands_ignores_timeouts() {
        let mut transport =
            MockPtyTransport::with_read_steps([ReadStep::Error(io::ErrorKind::TimedOut)]);
        let mut simulator = MpptControllerSimulator::default();
        let mut pending_command = Vec::new();

        read_pending_commands(&mut transport, &mut simulator, &mut pending_command)
            .expect("timeout reads should be ignored");

        assert!(pending_command.is_empty());
    }

    #[test]
    fn read_pending_commands_applies_valid_commands() {
        let mut transport = MockPtyTransport::with_read_steps([ReadStep::Bytes(b"LON\n".to_vec())]);
        let mut simulator = MpptControllerSimulator::default();
        let mut pending_command = Vec::new();

        read_pending_commands(&mut transport, &mut simulator, &mut pending_command)
            .expect("valid commands should be applied");

        assert!(simulator.next_frame().ends_with(":1"));
    }

    #[test]
    fn read_pending_commands_rejects_invalid_utf8() {
        let mut transport = MockPtyTransport::with_read_steps([ReadStep::Bytes(vec![0xFF, b'\n'])]);
        let mut simulator = MpptControllerSimulator::default();
        let mut pending_command = Vec::new();

        let err = read_pending_commands(&mut transport, &mut simulator, &mut pending_command)
            .expect_err("invalid UTF-8 commands should fail");

        assert_eq!(err.kind(), io::ErrorKind::InvalidData);
    }

    #[test]
    fn read_pending_commands_propagates_non_timeout_errors() {
        let mut transport =
            MockPtyTransport::with_read_steps([ReadStep::Error(io::ErrorKind::BrokenPipe)]);
        let mut simulator = MpptControllerSimulator::default();
        let mut pending_command = Vec::new();

        let err = read_pending_commands(&mut transport, &mut simulator, &mut pending_command)
            .expect_err("non-timeout read errors should be returned");

        assert_eq!(err.kind(), io::ErrorKind::BrokenPipe);
    }

    #[test]
    fn simulator_thread_wrapper_handles_write_failures() {
        let transport = MockPtyTransport::failing_on_write(io::ErrorKind::BrokenPipe);
        let (_shutdown_tx, shutdown_rx) = mpsc::channel();

        run_pty_simulator_thread(transport, Duration::from_millis(10), shutdown_rx);
    }

    #[test]
    fn tty_backed_simulator_thread_exits_on_invalid_commands() {
        let _guard = PTY_TEST_LOCK
            .lock()
            .expect("pty test lock should be available");
        let (mut master, mut slave) = TTYPort::pair().expect("tty pair should be created");
        master
            .set_timeout(Duration::from_millis(20))
            .expect("master timeout should be configurable");
        let (_shutdown_tx, shutdown_rx) = mpsc::channel();

        let simulator_thread = std::thread::spawn(move || {
            run_pty_simulator_thread(master, Duration::from_millis(10), shutdown_rx);
        });

        let mut frame_buffer = [0u8; 256];
        let _ = slave
            .read(&mut frame_buffer)
            .expect("frame read should succeed");
        slave
            .write_all(&[0xFF, b'\n'])
            .expect("invalid commands should be written");

        simulator_thread
            .join()
            .expect("simulator thread should stop after invalid input");
    }

    #[test]
    fn mock_transport_records_successful_writes_and_flushes() {
        let mut transport = MockPtyTransport::default();

        transport
            .write_all(b"frame")
            .expect("mock writes should succeed");
        transport.flush().expect("mock flushes should succeed");

        assert_eq!(transport.writes, b"frame");
    }

    #[test]
    fn serial_errors_are_mapped_to_io_errors() {
        let error = serialport::Error::new(ErrorKind::NoDevice, "missing port");
        let mapped = to_io_error(error);

        assert_eq!(mapped.kind(), io::ErrorKind::Other);
        assert!(mapped.to_string().contains("missing port"));
    }
}
