mod database;
mod datapoint;
mod load_toggle_switch;
mod serial_data_logger;

#[macro_use]
extern crate log;
extern crate simplelog;
use simplelog::{
    ColorChoice, CombinedLogger, Config, ConfigBuilder, LevelFilter, TermLogger, TerminalMode,
    WriteLogger,
};

use datapoint::DataPoint;
use load_toggle_switch::LoadToggleSwitch;
use serial_data_logger::SerialDatalogger;

use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, MouseEventKind},
    execute,
    terminal::{
        disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen, SetTitle,
    },
};
use std::{
    error::Error,
    fs::File,
    io,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc, Mutex,
    },
    thread::{self, sleep},
    time::Duration,
};
use tui::{
    backend::{Backend, CrosstermBackend},
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    widgets::{Block, BorderType, Borders, Cell, List, ListItem, ListState, Row, Table},
    Frame, Terminal,
};

type TermType = Terminal<CrosstermBackend<std::io::Stdout>>;
type TermResult = Result<Terminal<CrosstermBackend<std::io::Stdout>>, Box<dyn Error>>;

const LOGFILE_PATH: &str = "solar-rust.log";
const APP_NAME: &str = "Solar Tracer";

fn main() -> Result<(), Box<dyn Error>> {
    setup_logging()?;
    info!("Application Start");

    let ports = SerialDatalogger::get_comms();
    let mut terminal = setup_terminal()?;

    let mut port_list_state = ListState::default();
    port_list_state.select(Some(0));

    info!("Displaying serial ports.");
    let should_continue = display_ports(&mut terminal, &ports, &mut port_list_state)?;

    if should_continue {
        let port = match port_list_state.selected() {
            Some(p) => p,
            None => {
                error!("Not a valid port.");
                panic!("Not a valid port.")
            }
        };
        let selected_port = &ports[port];
        let res = run_app(&mut terminal, selected_port);
        if let Err(err) = res {
            error!("{:?}", err);
        }
    }

    info!("Cleaning up Terminal.");
    let err = cleanup_terminal(&mut terminal);
    if let Err(err) = err {
        error!("{:?}", err);
    }
    info!("Application End");
    Ok(())
}

fn setup_terminal() -> TermResult {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(
        stdout,
        EnterAlternateScreen,
        EnableMouseCapture,
        SetTitle(APP_NAME),
    )?;
    let backend = CrosstermBackend::new(stdout);
    Terminal::new(backend).map_err(std::convert::Into::into)
}

fn cleanup_terminal(terminal: &mut TermType) -> Result<(), Box<dyn Error>> {
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;
    Ok(())
}

fn setup_logging() -> Result<(), Box<dyn Error>> {
    let mut conf = ConfigBuilder::new();
    //conf.set_line_ending(LineEnding::Crlf);
    CombinedLogger::init(vec![
        TermLogger::new(
            LevelFilter::Warn,
            Config::default(),
            TerminalMode::Mixed,
            ColorChoice::Auto,
        ),
        WriteLogger::new(
            LevelFilter::Info,
            conf.build(),
            File::create(LOGFILE_PATH).unwrap(),
        ),
    ])
    .map_err(std::convert::Into::into)
}

fn display_ports<B: Backend>(
    terminal: &mut Terminal<B>,
    ports: &[String],
    port_list_state: &mut ListState,
) -> io::Result<bool> {
    for (i, p) in ports.iter().enumerate() {
        info!("{i}: {p:?}");
    }

    loop {
        let _ = terminal.draw(|f| init_ui(f, ports.to_vec(), port_list_state));
        if crossterm::event::poll(Duration::from_micros(100))? {
            if let Event::Key(key) = event::read()? {
                if let KeyCode::Enter = key.code {
                    info!("User selected: {}", port_list_state.selected().unwrap());
                    return Ok(true);
                }
                if let KeyCode::Char('q') = key.code {
                    return Ok(false);
                }
                if let KeyCode::Up = key.code {
                    info!("User action: {:?}", key.code);
                    if let Some(selected) = port_list_state.selected() {
                        let num_ports = ports.len();
                        if selected > 0 {
                            port_list_state.select(Some(selected - 1));
                        } else {
                            port_list_state.select(Some(num_ports - 1));
                        }
                    }
                }
                if let KeyCode::Down = key.code {
                    info!("User action: {:?}", key.code);
                    if let Some(selected) = port_list_state.selected() {
                        let num_ports = ports.len();
                        if selected >= num_ports - 1 {
                            port_list_state.select(Some(0));
                        } else {
                            port_list_state.select(Some(selected + 1));
                        }
                    }
                }
            }
        }
    }
}

fn run_app<B: Backend>(terminal: &mut Terminal<B>, selected_port: &String) -> io::Result<()> {
    let (rx, tx) = mpsc::channel();
    let (bg_tx_input, bg_rx_input) = mpsc::channel();
    let mut data_logger = SerialDatalogger::new(selected_port.to_string());
    let _ = data_logger.read_serial_datapoint(); //throw away read to ensure device is ready
    let initial_dp = data_logger.read_datapoint()?;
    let load_switch = Arc::new(Mutex::new(LoadToggleSwitch::new(
        initial_dp.get_load_onoff() > 0.0,
        ("ON", "OFF"),
    )));
    let running = Arc::new(AtomicBool::new(true));
    let builder = thread::Builder::new()
        .name("datalogger".into())
        .stack_size(1024 * 1024); //1MB
    let task = {
        let mut error_count: u64 = 0;
        let running = Arc::clone(&running);
        let selected_port_copy = String::from(selected_port);
        move || {
            while running.load(Ordering::SeqCst) {
                let datapoint = match data_logger.read_datapoint() {
                    Ok(d) => d,
                    Err(_e) => {
                        error_count += 1;
                        if error_count >= 5 {
                            error_count = 0;
                            info!(
                                "Failed to read 5 datapoints, attempting to reconnect in 1 second."
                            );
                            std::thread::sleep(Duration::from_secs(1));
                            data_logger = SerialDatalogger::new(selected_port_copy.clone());
                        }
                        DataPoint::default()
                    }
                };
                match rx.send(datapoint) {
                    Ok(_) => {}
                    Err(e) => error!("{}", e),
                }
                sleep(Duration::from_secs(1));
                match bg_rx_input.recv_timeout(Duration::from_micros(1000)) {
                    Ok(msg) => {
                        if msg {
                            data_logger.load_on();
                        } else {
                            data_logger.load_off();
                        }
                    }
                    Err(_e) => {}
                };
            }
        }
    };
    let _handle = builder
        .spawn(task)
        .expect("Error: creating data logging thread failed.");
    let mut current_dp = DataPoint::default();
    let input_thread = {
        let running = Arc::clone(&running);
        let load_switch = Arc::clone(&load_switch);
        let bg_tx = bg_tx_input.clone();
        move || {
            while running.load(Ordering::SeqCst) {
                match event::read().unwrap() {
                    Event::Key(q) => {
                        if let KeyCode::Char('q') = q.code {
                            running.store(false, Ordering::SeqCst);
                        }
                    }
                    Event::Mouse(me) => {
                        if let MouseEventKind::Down(_) = me.kind {
                            if me.row == 1 && me.column <= 10 {
                                if load_switch.lock().unwrap().is_on {
                                    load_switch.lock().unwrap().is_on = false;
                                    bg_tx.send(load_switch.lock().unwrap().is_on).unwrap();
                                } else {
                                    load_switch.lock().unwrap().is_on = true;
                                    bg_tx.send(load_switch.lock().unwrap().is_on).unwrap();
                                }
                            }
                        }
                    }
                    Event::FocusGained => {}
                    Event::FocusLost => {}
                    Event::Paste(_) => {}
                    Event::Resize(_, _) => {}
                }
            }
        }
    };
    let input_builder = thread::Builder::new()
        .name("input".into())
        .stack_size(1024 * 1024); //1MB
    let _handle = input_builder
        .spawn(input_thread)
        .expect("Error: creating input thread failed.");
    while running.load(Ordering::SeqCst) {
        current_dp = match tx.recv_timeout(Duration::from_millis(25)) {
            Ok(v) => v,
            Err(_e) => current_dp,
        };
        terminal.draw(|f| ui(f, current_dp, Arc::clone(&load_switch)))?;
    }
    Ok(())
}

fn init_ui<B: Backend>(f: &mut Frame<B>, ports: Vec<String>, port_list_state: &mut ListState) {
    let size = f.size();
    let block = Block::default()
        .borders(Borders::ALL)
        .title("Select Port")
        .title_alignment(Alignment::Center)
        .border_type(BorderType::Rounded);
    f.render_widget(block, size);
    let port_items: Vec<ListItem<'_>> = ports.iter().map(|f| ListItem::new(f.as_str())).collect();
    let port_list = List::new(port_items)
        .block(
            Block::default()
                .title("Port Selection (q to exit)")
                .title_alignment(Alignment::Center)
                .borders(Borders::ALL),
        )
        .style(Style::default().fg(Color::White))
        .highlight_style(Style::default().add_modifier(Modifier::ITALIC))
        .highlight_symbol(">>");
    f.render_stateful_widget(port_list, size, port_list_state);
}

fn ui<B: Backend>(
    f: &mut Frame<B>,
    datapoint: DataPoint,
    load_switch: Arc<Mutex<LoadToggleSwitch>>,
) {
    let size = f.size();
    let block = Block::default()
        .borders(Borders::ALL)
        .title(format!("{}{}", APP_NAME, ", q to quit"))
        .title_alignment(Alignment::Center)
        .border_type(BorderType::Rounded);
    f.render_widget(block, size);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(4)
        .constraints([Constraint::Percentage(100), Constraint::Percentage(100)].as_ref())
        .split(f.size());

    let top_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(100), Constraint::Percentage(100)].as_ref())
        .split(chunks[0]);
    let load = if datapoint.get_load_onoff() < 1.0 {
        "Off"
    } else {
        "On"
    };
    let load_current = datapoint.get_load_current().to_string();
    let battery_voltage = datapoint.get_battery_voltage().to_string();
    let battery_temp = datapoint.get_battery_temp().to_string();
    let pv_voltage = datapoint.get_pv_voltage().to_string();
    let charging = if datapoint.get_charging() < 1.0 {
        "No"
    } else {
        "Yes"
    };
    let charge_current = datapoint.get_charge_current().to_string();
    let over_discharge = datapoint.get_over_discharge().to_string();
    let battery_max = datapoint.get_battery_max().to_string();
    let battery_full = if datapoint.get_battery_full() < 1.0 {
        "No"
    } else {
        "Yes"
    };
    let time = datapoint.get_time_formatted();
    let table = Table::new(vec![
        Row::new(vec![
            Cell::from("Load: ").style(Style::default().fg(Color::Green)),
            Cell::from(load),
        ]),
        Row::new(vec![
            Cell::from("Load Current: ").style(Style::default().fg(Color::Green)),
            Cell::from(load_current),
        ]),
        Row::new(vec![
            Cell::from("Battery Voltage: ").style(Style::default().fg(Color::Green)),
            Cell::from(battery_voltage),
        ]),
        Row::new(vec![
            Cell::from("Battery Full: ").style(Style::default().fg(Color::Green)),
            Cell::from(battery_full),
        ]),
        Row::new(vec![
            Cell::from("Battery Temp: ").style(Style::default().fg(Color::Green)),
            Cell::from(battery_temp),
        ]),
        Row::new(vec![
            Cell::from("PV Voltage: ").style(Style::default().fg(Color::Green)),
            Cell::from(pv_voltage),
        ]),
        Row::new(vec![
            Cell::from("Charging: ").style(Style::default().fg(Color::Green)),
            Cell::from(charging),
        ]),
        Row::new(vec![
            Cell::from("Charge Current: ").style(Style::default().fg(Color::Green)),
            Cell::from(charge_current),
        ]),
        Row::new(vec![
            Cell::from("Over Discharge: ").style(Style::default().fg(Color::Green)),
            Cell::from(over_discharge),
        ]),
        Row::new(vec![
            Cell::from("Battery Max: ").style(Style::default().fg(Color::Green)),
            Cell::from(battery_max),
        ]),
        Row::new(vec![
            Cell::from("Timestamp: ").style(Style::default().fg(Color::Green)),
            Cell::from(time),
        ]),
    ])
    .style(Style::default().fg(Color::White))
    .block(Block::default().title("MPPT Data"))
    .widths(&[
        Constraint::Length(25),
        Constraint::Length(50),
        Constraint::Length(10),
    ])
    .column_spacing(1);
    f.render_widget(table, top_chunks[0]);
    let area = Rect::new(size.x, size.y, 10, 2);
    let button = load_switch.lock().unwrap().clone();
    f.render_widget(button, area);
}
