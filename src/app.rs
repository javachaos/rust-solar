//! Terminal UI runtime for selecting a controller port, reading telemetry, and rendering
//! a live dashboard.
//!
//! Most of the interesting wiring happens here:
//!
//! - port selection and controller construction
//! - background controller polling and reconnect behavior
//! - background input handling for keyboard and mouse shortcuts
//! - persistence of incoming datapoints to SQLite
//! - dashboard rendering for the current controller state

use std::{
    error::Error,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc::{Receiver, Sender},
    },
};

use crossterm::event::{Event, KeyCode, MouseEvent, MouseEventKind};
use log::warn;
use tui::{
    backend::Backend,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, BorderType, Borders, Cell, List, ListItem, ListState, Row, Table},
    Frame,
};

#[cfg(not(test))]
use tui::{backend::CrosstermBackend, Terminal};

use crate::{
    database::Database,
    datapoint::DataPoint,
    load_toggle_switch::LoadToggleSwitch,
    serial_data_logger::{ControllerCommand, ControllerError, MpptController},
};

#[cfg(not(test))]
use std::{
    env,
    fs::File,
    io,
    sync::{
        mpsc::{self, RecvTimeoutError},
        Arc,
    },
    thread::{self, JoinHandle},
    time::Duration,
};

#[cfg(not(test))]
use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture},
    execute,
    terminal::{
        disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen, SetTitle,
    },
};

#[cfg(not(test))]
use log::{error, info};

#[cfg(not(test))]
use simplelog::{
    ColorChoice, CombinedLogger, Config, LevelFilter, TermLogger, TerminalMode, WriteLogger,
};

#[cfg(not(test))]
use crate::{
    controller_simulator::{SimulatedSerialTransport, SIMULATOR_PORT_NAME},
    serial_data_logger::SerialDatalogger,
};

#[cfg(all(not(test), unix))]
use crate::pty_controller_harness::PTY_PORT_PREFIX;

/// Result type used by the app runtime and exported from the crate root.
pub type AppResult<T> = Result<T, Box<dyn Error>>;

#[cfg(not(test))]
type TermType = Terminal<CrosstermBackend<std::io::Stdout>>;

#[cfg(not(test))]
const LOGFILE_PATH: &str = "solar-rust.log";
const APP_NAME: &str = "Solar Tracer";
#[cfg(not(test))]
const CONTROLLER_ERROR_THRESHOLD: u64 = 5;
#[cfg(not(test))]
const CONTROLLER_READ_INTERVAL: Duration = Duration::from_secs(1);
#[cfg(not(test))]
const CONTROLLER_RECONNECT_DELAY: Duration = Duration::from_secs(1);
#[cfg(not(test))]
const DATA_POLL_INTERVAL: Duration = Duration::from_millis(25);
#[cfg(not(test))]
const INPUT_POLL_INTERVAL: Duration = Duration::from_millis(100);
const LOAD_SWITCH_WIDTH: u16 = 10;
const LOAD_SWITCH_HEIGHT: u16 = 3;

#[cfg(not(test))]
/// Starts the interactive terminal application.
///
/// The runtime presents a port-selection screen, connects to the chosen controller,
/// then keeps the dashboard, controller polling thread, input thread, and SQLite logger
/// in sync until the user quits.
pub fn run() -> AppResult<()> {
    setup_logging()?;
    info!("Application Start");

    let ports = available_ports();
    let mut terminal = TerminalSession::new()?;
    let mut port_list_state = ListState::default();
    port_list_state.select((!ports.is_empty()).then_some(0));

    info!("Displaying serial ports.");
    if display_ports(terminal.terminal_mut(), &ports, &mut port_list_state)? {
        if let Some(port) = selected_port(&ports, &port_list_state) {
            if let Err(err) = run_app(terminal.terminal_mut(), port) {
                error!("{err}");
            }
        }
    }

    info!("Application End");
    Ok(())
}

#[cfg(test)]
/// Test-only stub that keeps the public crate entrypoint callable without launching the UI.
pub fn run() -> AppResult<()> {
    Ok(())
}

#[cfg(not(test))]
struct TerminalSession {
    terminal: TermType,
}

#[cfg(not(test))]
impl TerminalSession {
    fn new() -> AppResult<Self> {
        enable_raw_mode()?;
        let mut stdout = io::stdout();
        execute!(
            stdout,
            EnterAlternateScreen,
            EnableMouseCapture,
            SetTitle(APP_NAME),
        )?;
        let backend = CrosstermBackend::new(stdout);
        let terminal = Terminal::new(backend)?;

        Ok(Self { terminal })
    }

    fn terminal_mut(&mut self) -> &mut TermType {
        &mut self.terminal
    }
}

#[cfg(not(test))]
impl Drop for TerminalSession {
    fn drop(&mut self) {
        if let Err(err) = disable_raw_mode() {
            eprintln!("failed to disable raw mode: {err}");
        }

        if let Err(err) = execute!(
            self.terminal.backend_mut(),
            LeaveAlternateScreen,
            DisableMouseCapture
        ) {
            eprintln!("failed to restore terminal screen: {err}");
        }

        if let Err(err) = self.terminal.show_cursor() {
            eprintln!("failed to show terminal cursor: {err}");
        }
    }
}

#[cfg(not(test))]
fn setup_logging() -> AppResult<()> {
    CombinedLogger::init(vec![
        TermLogger::new(
            LevelFilter::Error,
            Config::default(),
            TerminalMode::Mixed,
            ColorChoice::Auto,
        ),
        WriteLogger::new(
            LevelFilter::Info,
            Config::default(),
            File::create(LOGFILE_PATH)?,
        ),
    ])
    .map_err(std::convert::Into::into)
}

#[cfg(not(test))]
fn available_ports() -> Vec<String> {
    let mut ports = match SerialDatalogger::get_comms() {
        Ok(ports) => ports,
        Err(err) => {
            warn!("Failed to enumerate serial ports: {err}");
            Vec::new()
        }
    };

    ports.extend(extra_ports_from_env());
    ports.push(SIMULATOR_PORT_NAME.to_string());
    ports
}

#[cfg(not(test))]
fn extra_ports_from_env() -> Vec<String> {
    env::var("SOLAR_EXTRA_PORTS")
        .map(|value| {
            value
                .split(',')
                .map(str::trim)
                .filter(|port| !port.is_empty())
                .map(ToOwned::to_owned)
                .collect()
        })
        .unwrap_or_default()
}

#[cfg(not(test))]
fn display_ports<B: Backend>(
    terminal: &mut Terminal<B>,
    ports: &[String],
    port_list_state: &mut ListState,
) -> io::Result<bool> {
    for (index, port) in ports.iter().enumerate() {
        info!("{index}: {port}");
    }

    loop {
        terminal.draw(|frame| draw_port_selection(frame, ports, port_list_state))?;

        if event::poll(INPUT_POLL_INTERVAL)? {
            if let Event::Key(key) = event::read()? {
                if let Some(should_continue) =
                    handle_port_selection_key(key.code, ports.len(), port_list_state)
                {
                    return Ok(should_continue);
                }
            }
        }
    }
}

#[cfg(not(test))]
/// Runs the main dashboard loop after a controller has been selected.
///
/// The selected controller is primed once, its current state is persisted immediately,
/// and then background threads are spawned for controller polling and user input.
fn run_app<B: Backend>(terminal: &mut Terminal<B>, selected_port: &str) -> AppResult<()> {
    let (data_tx, data_rx) = mpsc::channel();
    let (command_tx, command_rx) = mpsc::channel();

    let mut controller = create_controller(selected_port)?;
    controller.prime()?;
    let mut current_datapoint = controller.read_datapoint()?;

    let mut database = Database::open_default()?;
    database.add_datapoint(current_datapoint)?;

    let load_state = Arc::new(AtomicBool::new(current_datapoint.is_load_enabled()));
    let running = Arc::new(AtomicBool::new(true));

    let controller_handle = spawn_controller_thread(
        controller,
        selected_port.to_string(),
        Arc::clone(&running),
        data_tx,
        command_rx,
    )?;
    let input_handle = spawn_input_thread(
        Arc::clone(&running),
        Arc::clone(&load_state),
        command_tx.clone(),
    )?;

    while running.load(Ordering::SeqCst) {
        match data_rx.recv_timeout(DATA_POLL_INTERVAL) {
            Ok(datapoint) => {
                process_datapoint(
                    &mut database,
                    &load_state,
                    &mut current_datapoint,
                    datapoint,
                )?;
            }
            Err(RecvTimeoutError::Timeout) => {}
            Err(RecvTimeoutError::Disconnected) => {
                warn!("Controller thread disconnected.");
                running.store(false, Ordering::SeqCst);
            }
        }

        terminal.draw(|frame| {
            draw_dashboard(frame, current_datapoint, load_state.load(Ordering::SeqCst))
        })?;
    }

    drop(command_tx);
    join_thread("input", input_handle);
    join_thread("controller", controller_handle);
    database.flush()?;

    Ok(())
}

#[cfg(not(test))]
/// Creates a controller implementation for the selected port name.
///
/// Supported modes are:
///
/// - the in-process simulator listed in the UI
/// - Unix PTY-backed simulator ports prefixed with `pty:`
/// - real serial devices opened through `serialport`
fn create_controller(selected_port: &str) -> Result<Box<dyn MpptController>, ControllerError> {
    if selected_port == SIMULATOR_PORT_NAME {
        return Ok(Box::new(SerialDatalogger::from_port(
            SimulatedSerialTransport::default(),
        )));
    }

    #[cfg(unix)]
    if let Some(port_name) = selected_port.strip_prefix(PTY_PORT_PREFIX) {
        return Ok(Box::new(SerialDatalogger::connect_pty(port_name)?));
    }

    Ok(Box::new(SerialDatalogger::connect(selected_port)?))
}

#[cfg(not(test))]
/// Spawns the background controller loop that applies queued commands and emits datapoints.
///
/// Reconnect attempts are triggered after a configurable number of consecutive controller
/// errors so a temporarily unavailable device does not permanently stop the dashboard.
fn spawn_controller_thread(
    mut controller: Box<dyn MpptController>,
    selected_port: String,
    running: Arc<AtomicBool>,
    data_tx: Sender<DataPoint>,
    command_rx: Receiver<ControllerCommand>,
) -> io::Result<JoinHandle<()>> {
    thread::Builder::new()
        .name("controller".into())
        .stack_size(1024 * 1024)
        .spawn(move || {
            let mut consecutive_errors = 0;

            while running.load(Ordering::SeqCst) {
                match pump_controller(&mut *controller, &command_rx) {
                    Ok(datapoint) => {
                        consecutive_errors = 0;
                        if data_tx.send(datapoint).is_err() {
                            running.store(false, Ordering::SeqCst);
                            break;
                        }
                    }
                    Err(err) => {
                        warn!("{err}");
                        consecutive_errors += 1;

                        if consecutive_errors >= CONTROLLER_ERROR_THRESHOLD {
                            warn!(
                                "Failed to read {} datapoints, attempting reconnect in 1 second.",
                                CONTROLLER_ERROR_THRESHOLD
                            );
                            thread::sleep(CONTROLLER_RECONNECT_DELAY);

                            match create_controller(&selected_port) {
                                Ok(mut replacement) => {
                                    if let Err(err) = replacement.prime() {
                                        warn!("Failed to prime replacement controller: {err}");
                                    }
                                    controller = replacement;
                                    consecutive_errors = 0;
                                }
                                Err(err) => warn!("Reconnect failed: {err}"),
                            }
                        }
                    }
                }

                thread::sleep(CONTROLLER_READ_INTERVAL);
            }
        })
}

#[cfg(not(test))]
/// Spawns the background input loop that watches for keyboard and mouse events.
fn spawn_input_thread(
    running: Arc<AtomicBool>,
    load_state: Arc<AtomicBool>,
    command_tx: Sender<ControllerCommand>,
) -> io::Result<JoinHandle<()>> {
    thread::Builder::new()
        .name("input".into())
        .stack_size(1024 * 1024)
        .spawn(move || {
            while running.load(Ordering::SeqCst) {
                match event::poll(INPUT_POLL_INTERVAL) {
                    Ok(true) => match event::read() {
                        Ok(input) => handle_input_event(input, &running, &load_state, &command_tx),
                        Err(err) => warn!("Failed to read input event: {err}"),
                    },
                    Ok(false) => {}
                    Err(err) => warn!("Failed to poll for input events: {err}"),
                }
            }
        })
}

/// Applies a single terminal input event to the running application state.
fn handle_input_event(
    input: Event,
    running: &AtomicBool,
    load_state: &AtomicBool,
    command_tx: &Sender<ControllerCommand>,
) {
    match input {
        Event::Key(key) => handle_runtime_key(key.code, running, load_state, command_tx),
        Event::Mouse(mouse_event) if is_load_click(&mouse_event) => {
            toggle_load_state(load_state, command_tx);
        }
        Event::FocusGained
        | Event::FocusLost
        | Event::Mouse(_)
        | Event::Paste(_)
        | Event::Resize(_, _) => {}
    }
}

fn handle_runtime_key(
    key_code: KeyCode,
    running: &AtomicBool,
    load_state: &AtomicBool,
    command_tx: &Sender<ControllerCommand>,
) {
    match key_code {
        KeyCode::Char('q') => running.store(false, Ordering::SeqCst),
        KeyCode::Char('l') | KeyCode::Char(' ') => toggle_load_state(load_state, command_tx),
        _ => {}
    }
}

/// Updates the locally cached load state and queues a controller command to make it real.
fn toggle_load_state(load_state: &AtomicBool, command_tx: &Sender<ControllerCommand>) {
    let next_state = !load_state.load(Ordering::SeqCst);
    load_state.store(next_state, Ordering::SeqCst);

    if let Err(err) = command_tx.send(ControllerCommand::SetLoad(next_state)) {
        warn!("Failed to send load toggle command: {err}");
    }
}

fn is_load_click(mouse_event: &MouseEvent) -> bool {
    matches!(mouse_event.kind, MouseEventKind::Down(_))
        && mouse_event.column < LOAD_SWITCH_WIDTH
        && mouse_event.row < LOAD_SWITCH_HEIGHT
}

/// Persists the latest datapoint and updates the UI-facing state snapshot.
fn process_datapoint(
    database: &mut Database,
    load_state: &AtomicBool,
    current_datapoint: &mut DataPoint,
    datapoint: DataPoint,
) -> rusqlite::Result<()> {
    load_state.store(datapoint.is_load_enabled(), Ordering::SeqCst);
    database.add_datapoint(datapoint)?;
    *current_datapoint = datapoint;
    Ok(())
}

/// Applies any queued controller commands before reading the next datapoint.
///
/// This ordering keeps the simulator and real hardware paths consistent: a pending load
/// toggle is sent before the next telemetry frame is consumed.
fn pump_controller(
    controller: &mut dyn MpptController,
    command_rx: &Receiver<ControllerCommand>,
) -> Result<DataPoint, ControllerError> {
    apply_pending_controller_commands(controller, command_rx)?;
    controller.read_datapoint()
}

/// Drains queued controller commands without blocking.
fn apply_pending_controller_commands(
    controller: &mut dyn MpptController,
    command_rx: &Receiver<ControllerCommand>,
) -> Result<(), ControllerError> {
    while let Ok(command) = command_rx.try_recv() {
        match command {
            ControllerCommand::SetLoad(enabled) => controller.set_load_enabled(enabled)?,
        }
    }

    Ok(())
}

#[cfg(not(test))]
fn join_thread(name: &str, handle: JoinHandle<()>) {
    if let Err(err) = handle.join() {
        warn!("Failed to join {name} thread: {err:?}");
    }
}

/// Handles navigation and selection actions in the initial port-selection screen.
fn handle_port_selection_key(
    key_code: KeyCode,
    port_count: usize,
    port_list_state: &mut ListState,
) -> Option<bool> {
    match key_code {
        KeyCode::Enter => port_list_state.selected().map(|_| true),
        KeyCode::Char('q') => Some(false),
        KeyCode::Up => {
            move_selection(port_list_state, port_count, SelectionDirection::Up);
            None
        }
        KeyCode::Down => {
            move_selection(port_list_state, port_count, SelectionDirection::Down);
            None
        }
        _ => None,
    }
}

/// Moves the currently highlighted port selection, wrapping at both ends.
fn move_selection(
    port_list_state: &mut ListState,
    port_count: usize,
    direction: SelectionDirection,
) {
    if port_count == 0 {
        port_list_state.select(None);
        return;
    }

    let current = port_list_state.selected().unwrap_or(0);
    let next = match direction {
        SelectionDirection::Up => {
            if current == 0 {
                port_count - 1
            } else {
                current - 1
            }
        }
        SelectionDirection::Down => {
            if current + 1 >= port_count {
                0
            } else {
                current + 1
            }
        }
    };

    port_list_state.select(Some(next));
}

fn selected_port<'a>(ports: &'a [String], port_list_state: &ListState) -> Option<&'a str> {
    port_list_state
        .selected()
        .and_then(|selected| ports.get(selected))
        .map(String::as_str)
}

/// Draws the initial port-selection view.
fn draw_port_selection<B: Backend>(
    frame: &mut Frame<B>,
    ports: &[String],
    port_list_state: &mut ListState,
) {
    let size = frame.size();
    let block = Block::default()
        .borders(Borders::ALL)
        .title("Select Port")
        .title_alignment(Alignment::Center)
        .border_type(BorderType::Rounded);
    frame.render_widget(block, size);

    let port_items: Vec<ListItem<'_>> = if ports.is_empty() {
        vec![ListItem::new("No serial ports available")]
    } else {
        ports
            .iter()
            .map(|port| ListItem::new(port.as_str()))
            .collect()
    };

    let port_list = List::new(port_items)
        .block(
            Block::default()
                .title("Port Selection (Enter to connect, q to exit)")
                .title_alignment(Alignment::Center)
                .borders(Borders::ALL),
        )
        .style(Style::default().fg(Color::White))
        .highlight_style(Style::default().add_modifier(Modifier::ITALIC))
        .highlight_symbol(">>");

    frame.render_stateful_widget(port_list, size, port_list_state);
}

/// Draws the live controller dashboard for the most recent datapoint.
fn draw_dashboard<B: Backend>(frame: &mut Frame<B>, datapoint: DataPoint, load_enabled: bool) {
    let size = frame.size();
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!("{APP_NAME}, q to quit, l/space to toggle load"))
        .title_alignment(Alignment::Center)
        .border_type(BorderType::Rounded);
    frame.render_widget(block, size);

    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(4)
        .constraints([Constraint::Percentage(100), Constraint::Percentage(100)])
        .split(frame.size());

    let top_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(100), Constraint::Percentage(100)])
        .split(chunks[0]);

    let table = Table::new(vec![
        Row::new(vec![
            Cell::from("Load").style(label_style()),
            Cell::from(format_toggle_label(load_enabled)),
        ]),
        Row::new(vec![
            Cell::from("Load Current").style(label_style()),
            Cell::from(format_measurement(datapoint.get_load_current(), " A")),
        ]),
        Row::new(vec![
            Cell::from("Battery Voltage").style(label_style()),
            Cell::from(format_measurement(datapoint.get_battery_voltage(), " V")),
        ]),
        Row::new(vec![
            Cell::from("Battery Full").style(label_style()),
            Cell::from(format_yes_no(datapoint.is_battery_full())),
        ]),
        Row::new(vec![
            Cell::from("Battery Temp").style(label_style()),
            Cell::from(format_measurement(datapoint.get_battery_temp(), " C")),
        ]),
        Row::new(vec![
            Cell::from("PV Voltage").style(label_style()),
            Cell::from(format_measurement(datapoint.get_pv_voltage(), " V")),
        ]),
        Row::new(vec![
            Cell::from("Charging").style(label_style()),
            Cell::from(format_yes_no(datapoint.is_charging())),
        ]),
        Row::new(vec![
            Cell::from("Charge Current").style(label_style()),
            Cell::from(format_measurement(datapoint.get_charge_current(), " A")),
        ]),
        Row::new(vec![
            Cell::from("Over Discharge").style(label_style()),
            Cell::from(format_measurement(datapoint.get_over_discharge(), " V")),
        ]),
        Row::new(vec![
            Cell::from("Battery Max").style(label_style()),
            Cell::from(format_measurement(datapoint.get_battery_max(), " V")),
        ]),
        Row::new(vec![
            Cell::from("Timestamp").style(label_style()),
            Cell::from(datapoint.get_time_formatted()),
        ]),
    ])
    .style(Style::default().fg(Color::White))
    .block(Block::default().title("MPPT Data"))
    .widths(&[Constraint::Length(25), Constraint::Length(50)])
    .column_spacing(1);
    frame.render_widget(table, top_chunks[0]);

    let load_switch = LoadToggleSwitch::new(load_enabled, ("ON", "OFF"));
    frame.render_widget(
        load_switch,
        Rect::new(size.x, size.y, LOAD_SWITCH_WIDTH, LOAD_SWITCH_HEIGHT),
    );
}

fn label_style() -> Style {
    Style::default().fg(Color::Green)
}

fn format_measurement(value: f64, suffix: &str) -> String {
    format!("{value:.2}{suffix}")
}

fn format_yes_no(value: bool) -> &'static str {
    if value {
        "Yes"
    } else {
        "No"
    }
}

fn format_toggle_label(value: bool) -> &'static str {
    if value {
        "On"
    } else {
        "Off"
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum SelectionDirection {
    Up,
    Down,
}

#[cfg(test)]
mod tests {
    use std::{
        env::temp_dir,
        fs,
        sync::{
            atomic::{AtomicBool, Ordering},
            mpsc,
        },
        time::{SystemTime, UNIX_EPOCH},
    };

    use crossterm::event::{Event, KeyCode, KeyModifiers, MouseButton, MouseEvent, MouseEventKind};
    use rusqlite::Connection;
    use tui::{backend::TestBackend, buffer::Buffer, widgets::ListState, Terminal};

    use super::{
        apply_pending_controller_commands, draw_dashboard, draw_port_selection, format_measurement,
        format_toggle_label, format_yes_no, handle_input_event, handle_port_selection_key,
        is_load_click, move_selection, process_datapoint, pump_controller, run, selected_port,
        toggle_load_state, SelectionDirection, LOAD_SWITCH_HEIGHT, LOAD_SWITCH_WIDTH,
    };
    use crate::{
        controller_simulator::{SimulatedController, SIMULATOR_PORT_NAME},
        database::Database,
        datapoint::DataPoint,
        serial_data_logger::{ControllerCommand, MpptController},
    };

    fn sample_datapoint(load_enabled: bool) -> DataPoint {
        DataPoint::from_values_with_timestamp(
            1_710_000_000,
            [
                12.5,
                18.2,
                if load_enabled { 3.2 } else { 0.0 },
                10.7,
                14.8,
                1.0,
                1.0,
                24.0,
                5.6,
                if load_enabled { 1.0 } else { 0.0 },
            ],
        )
    }

    fn temp_database_path(name: &str) -> std::path::PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("current time should be after UNIX epoch")
            .as_nanos();
        temp_dir().join(format!("rust-solar-app-{name}-{unique}.sqlite"))
    }

    fn buffer_lines(buffer: &Buffer) -> Vec<String> {
        (0..buffer.area.height)
            .map(|y| {
                (0..buffer.area.width)
                    .map(|x| buffer.get(x, y).symbol.clone())
                    .collect::<String>()
            })
            .collect()
    }

    #[test]
    fn selection_wraps_in_both_directions() {
        let mut state = ListState::default();
        state.select(Some(0));

        move_selection(&mut state, 3, SelectionDirection::Up);
        assert_eq!(state.selected(), Some(2));

        move_selection(&mut state, 3, SelectionDirection::Down);
        assert_eq!(state.selected(), Some(0));
    }

    #[test]
    fn run_is_a_noop_in_tests() {
        run().expect("test run should succeed");
    }

    #[test]
    fn handle_port_selection_key_supports_enter_and_quit() {
        let mut state = ListState::default();
        state.select(Some(1));

        assert_eq!(
            handle_port_selection_key(KeyCode::Enter, 3, &mut state),
            Some(true)
        );
        assert_eq!(
            handle_port_selection_key(KeyCode::Char('q'), 3, &mut state),
            Some(false)
        );
    }

    #[test]
    fn handle_port_selection_key_supports_navigation_and_ignores_other_keys() {
        let mut state = ListState::default();
        state.select(Some(1));

        assert_eq!(handle_port_selection_key(KeyCode::Up, 3, &mut state), None);
        assert_eq!(state.selected(), Some(0));

        assert_eq!(
            handle_port_selection_key(KeyCode::Down, 3, &mut state),
            None
        );
        assert_eq!(state.selected(), Some(1));

        assert_eq!(
            handle_port_selection_key(KeyCode::Char('x'), 3, &mut state),
            None
        );
        assert_eq!(state.selected(), Some(1));
    }

    #[test]
    fn selected_port_respects_current_selection() {
        let ports = vec!["COM1".to_string(), "Simulator".to_string()];
        let mut state = ListState::default();
        state.select(Some(1));

        assert_eq!(selected_port(&ports, &state), Some("Simulator"));
    }

    #[test]
    fn toggle_load_state_updates_atomic_flag_and_sends_command() {
        let load_state = AtomicBool::new(false);
        let (tx, rx) = mpsc::channel();

        toggle_load_state(&load_state, &tx);

        assert!(load_state.load(Ordering::SeqCst));
        assert_eq!(
            rx.recv().expect("command should be sent"),
            ControllerCommand::SetLoad(true)
        );
    }

    #[test]
    fn toggle_load_state_handles_disconnected_receivers() {
        let load_state = AtomicBool::new(false);
        let (tx, rx) = mpsc::channel::<ControllerCommand>();
        drop(rx);

        toggle_load_state(&load_state, &tx);

        assert!(load_state.load(Ordering::SeqCst));
    }

    #[test]
    fn mouse_click_detection_matches_load_button_area() {
        let click = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 1,
            row: 1,
            modifiers: crossterm::event::KeyModifiers::NONE,
        };
        let outside = MouseEvent {
            kind: MouseEventKind::Down(MouseButton::Left),
            column: 15,
            row: 5,
            modifiers: crossterm::event::KeyModifiers::NONE,
        };

        assert!(is_load_click(&click));
        assert!(!is_load_click(&outside));
    }

    #[test]
    fn handle_input_event_supports_mouse_clicks_and_noop_events() {
        let running = AtomicBool::new(true);
        let load_state = AtomicBool::new(false);
        let (tx, rx) = mpsc::channel();

        handle_input_event(
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: 1,
                row: 1,
                modifiers: KeyModifiers::NONE,
            }),
            &running,
            &load_state,
            &tx,
        );

        assert!(load_state.load(Ordering::SeqCst));
        assert_eq!(
            rx.recv().expect("mouse click should send a command"),
            ControllerCommand::SetLoad(true)
        );

        handle_input_event(
            Event::Mouse(MouseEvent {
                kind: MouseEventKind::Down(MouseButton::Left),
                column: LOAD_SWITCH_WIDTH + 1,
                row: LOAD_SWITCH_HEIGHT + 1,
                modifiers: KeyModifiers::NONE,
            }),
            &running,
            &load_state,
            &tx,
        );
        handle_input_event(Event::FocusGained, &running, &load_state, &tx);
        handle_input_event(Event::FocusLost, &running, &load_state, &tx);
        handle_input_event(
            Event::Paste("ignored".to_string()),
            &running,
            &load_state,
            &tx,
        );
        handle_input_event(Event::Resize(80, 24), &running, &load_state, &tx);

        assert!(load_state.load(Ordering::SeqCst));
        assert!(
            rx.try_recv().is_err(),
            "noop events should not send commands"
        );
    }

    #[test]
    fn handle_input_event_supports_quit_and_toggle_shortcuts() {
        let running = AtomicBool::new(true);
        let load_state = AtomicBool::new(false);
        let (tx, rx) = mpsc::channel();

        handle_input_event(
            crossterm::event::Event::Key(crossterm::event::KeyEvent::from(KeyCode::Char('l'))),
            &running,
            &load_state,
            &tx,
        );

        assert!(load_state.load(Ordering::SeqCst));
        assert_eq!(
            rx.recv().expect("command should be sent"),
            ControllerCommand::SetLoad(true)
        );

        handle_input_event(
            crossterm::event::Event::Key(crossterm::event::KeyEvent::from(KeyCode::Char('q'))),
            &running,
            &load_state,
            &tx,
        );

        assert!(!running.load(Ordering::SeqCst));
    }

    #[test]
    fn handle_input_event_ignores_unmapped_keys() {
        let running = AtomicBool::new(true);
        let load_state = AtomicBool::new(false);
        let (tx, rx) = mpsc::channel();

        handle_input_event(
            Event::Key(crossterm::event::KeyEvent::from(KeyCode::Char('x'))),
            &running,
            &load_state,
            &tx,
        );

        assert!(running.load(Ordering::SeqCst));
        assert!(!load_state.load(Ordering::SeqCst));
        assert!(
            rx.try_recv().is_err(),
            "unmapped keys should not send commands"
        );
    }

    #[test]
    fn pending_controller_commands_are_applied_before_reads() {
        let mut controller = SimulatedController::default();
        let (tx, rx) = mpsc::channel();
        tx.send(ControllerCommand::SetLoad(true))
            .expect("send should succeed");

        apply_pending_controller_commands(&mut controller, &rx)
            .expect("controller commands should apply");
        let datapoint = controller
            .read_datapoint()
            .expect("controller should produce a datapoint");

        assert!(datapoint.is_load_enabled());
    }

    #[test]
    fn process_datapoint_updates_state_and_persists_to_database() {
        let path = temp_database_path("process-datapoint");
        let mut database = Database::open_path(&path).expect("database should open");
        let load_state = AtomicBool::new(false);
        let mut current = sample_datapoint(false);
        let next = sample_datapoint(true);

        process_datapoint(&mut database, &load_state, &mut current, next)
            .expect("processing should succeed");
        database.flush().expect("flush should succeed");

        let connection = Connection::open(&path).expect("database should reopen");
        let count: i64 = connection
            .query_row("SELECT COUNT(*) FROM Data", [], |row| row.get(0))
            .expect("count query should succeed");

        assert_eq!(count, 1);
        assert!(load_state.load(Ordering::SeqCst));
        assert!(current.is_load_enabled());

        drop(connection);
        drop(database);
        fs::remove_file(path).expect("temporary database should be removable");
    }

    #[test]
    fn formatting_helpers_return_user_friendly_strings() {
        assert_eq!(format_measurement(12.345, " V"), "12.35 V");
        assert_eq!(format_yes_no(true), "Yes");
        assert_eq!(format_yes_no(false), "No");
        assert_eq!(format_toggle_label(true), "On");
        assert_eq!(format_toggle_label(false), "Off");
    }

    #[test]
    fn pump_controller_applies_commands_before_reading() {
        let mut controller = SimulatedController::default();
        let (tx, rx) = mpsc::channel();
        tx.send(ControllerCommand::SetLoad(true))
            .expect("send should succeed");

        let datapoint = pump_controller(&mut controller, &rx).expect("pump should succeed");

        assert!(datapoint.is_load_enabled());
    }

    #[test]
    fn move_selection_handles_empty_lists_and_non_wrapping_steps() {
        let mut empty_state = ListState::default();
        empty_state.select(Some(0));
        move_selection(&mut empty_state, 0, SelectionDirection::Down);
        assert_eq!(empty_state.selected(), None);

        let mut populated_state = ListState::default();
        populated_state.select(Some(2));
        move_selection(&mut populated_state, 4, SelectionDirection::Up);
        assert_eq!(populated_state.selected(), Some(1));

        move_selection(&mut populated_state, 4, SelectionDirection::Down);
        assert_eq!(populated_state.selected(), Some(2));
    }

    #[test]
    fn draw_port_selection_renders_available_ports() {
        let backend = TestBackend::new(60, 10);
        let mut terminal = Terminal::new(backend).expect("terminal should be created");
        let ports = vec!["COM1".to_string(), SIMULATOR_PORT_NAME.to_string()];
        let mut state = ListState::default();
        state.select(Some(1));

        terminal
            .draw(|frame| draw_port_selection(frame, &ports, &mut state))
            .expect("draw should succeed");

        let rendered = buffer_lines(terminal.backend().buffer()).join("\n");
        assert!(rendered.contains("Port Selection"));
        assert!(rendered.contains("COM1"));
        assert!(rendered.contains("Simulator (no hardware)"));
    }

    #[test]
    fn draw_port_selection_renders_empty_state() {
        let backend = TestBackend::new(60, 10);
        let mut terminal = Terminal::new(backend).expect("terminal should be created");
        let ports = Vec::new();
        let mut state = ListState::default();

        terminal
            .draw(|frame| draw_port_selection(frame, &ports, &mut state))
            .expect("draw should succeed");

        let rendered = buffer_lines(terminal.backend().buffer()).join("\n");
        assert!(rendered.contains("No serial ports available"));
    }

    #[test]
    fn draw_dashboard_renders_key_measurements() {
        let backend = TestBackend::new(80, 20);
        let mut terminal = Terminal::new(backend).expect("terminal should be created");

        terminal
            .draw(|frame| draw_dashboard(frame, sample_datapoint(true), true))
            .expect("draw should succeed");

        let rendered = buffer_lines(terminal.backend().buffer()).join("\n");
        assert!(rendered.contains("Solar Tracer"));
        assert!(rendered.contains("Battery Voltage"));
        assert!(rendered.contains("12.50 V"));
        assert!(rendered.contains("Load"));
        assert!(rendered.contains("On"));
    }
}
