mod database;
mod datapoint;
mod serial_data_logger;

#[macro_use]
extern crate log;
extern crate simplelog;
use simplelog::{ColorChoice, CombinedLogger, Config, LevelFilter, TermLogger, TerminalMode, WriteLogger};

use datapoint::DataPoint;
use serial_data_logger::SerialDatalogger;

use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use std::{
    error::Error,
    fs::File,
    io::{self, Write},
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    thread::{self, sleep},
    time::{Duration, Instant},
};
use tui::{
    backend::{Backend, CrosstermBackend},
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Style},
    widgets::{Block, BorderType, Borders, Cell, Row, Table},
    Frame, Terminal,
};

// These types were too long...
type TermType = Terminal<CrosstermBackend<std::io::Stdout>>;
type TermResult = Result<Terminal<CrosstermBackend<std::io::Stdout>>, Box<dyn Error>>;

const LOGFILE_PATH: &str = "solar-rust.log";

fn main() -> Result<(), Box<dyn Error>> {
    setup_logging()?;

    let ports = SerialDatalogger::get_comms();
    display_ports(&ports);

    let selected_port = select_port(&ports)?;

    let mut terminal = setup_terminal()?;
    let tick_rate = Duration::from_millis(250);
    let res = run_app(&mut terminal, selected_port, tick_rate);

    cleanup_terminal(&mut terminal)?;

    if let Err(err) = res {
        error!("{:?}", err);
    }

    Ok(())
}

fn setup_terminal() -> TermResult {
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
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
    CombinedLogger::init(vec![
        TermLogger::new(
            LevelFilter::Warn,
            Config::default(),
            TerminalMode::Mixed,
            ColorChoice::Auto,
        ),
        WriteLogger::new(
            LevelFilter::Info,
            Config::default(),
            File::create(LOGFILE_PATH).unwrap(),
        ),
    ])
    .map_err(std::convert::Into::into)
}

fn display_ports(ports: &[String]) {
    for (i, p) in ports.iter().enumerate() {
        println!("{i}: {p:?}");
        info!("{i}: {p:?}");
    }
}

fn select_port(ports: &[String]) -> Result<&String, Box<dyn Error>> {
    print!("Please select a port: ");
    io::stdout().flush()?;

    let mut port_index_str = String::new();
    io::stdin().read_line(&mut port_index_str)?;

    let port_index = port_index_str
        .trim()
        .parse::<usize>()
        .map_err(|e| format!("Invalid port index: {e}"))?;

    if port_index >= ports.len() {
        return Err("Invalid port index".into());
    }

    Ok(&ports[port_index])
}

fn run_app<B: Backend>(
    terminal: &mut Terminal<B>,
    selected_port: &String,
    tick_rate: Duration,
) -> io::Result<()> {
    let mut last_tick = Instant::now();
    let (rx, tx) = mpsc::channel();
    let mut data_logger = SerialDatalogger::new(selected_port.to_string());
    let running = Arc::new(AtomicBool::new(true));
    let builder = thread::Builder::new()
        .name("datalogger".into())
        .stack_size(1024 * 1024); //1MB
    let task = {
        let running = Arc::clone(&running);
        move || {
            while running.load(Ordering::SeqCst) {
                let datapoint = data_logger.read_datapoint();
                rx.send(datapoint).unwrap();
                sleep(Duration::from_secs(1));
            }
        }
    };
    let handle = builder
        .spawn(task)
        .expect("Error: creating data logging thread failed.");
    let mut current_dp = DataPoint::default();
    loop {
        current_dp = match tx.recv_timeout(Duration::from_micros(1000)) {
            Ok(v) => v,
            Err(_e) => current_dp,
        };
        terminal.draw(|f| ui(f, current_dp))?;

        let timeout = tick_rate
            .checked_sub(last_tick.elapsed())
            .unwrap_or_else(|| Duration::from_secs(0));
        if crossterm::event::poll(timeout)? {
            if let Event::Key(key) = event::read()? {
                if let KeyCode::Char('q') = key.code {
                    running.store(false, Ordering::SeqCst);
                    handle.join().unwrap();
                    return Ok(());
                }
            }
        }
        if last_tick.elapsed() >= tick_rate {
            last_tick = Instant::now();
        }
    }
}

fn ui<B: Backend>(f: &mut Frame<B>, datapoint: DataPoint) {
    let size = f.size();
    let block = Block::default()
        .borders(Borders::ALL)
        .title("Solar Tracer, q to quit")
        .title_alignment(Alignment::Center)
        .border_type(BorderType::Rounded);
    f.render_widget(block, size);
    let chunks = Layout::default()
        .direction(Direction::Vertical)
        .margin(4)
        .constraints([Constraint::Percentage(100), Constraint::Percentage(50)].as_ref())
        .split(f.size());

    let top_chunks = Layout::default()
        .direction(Direction::Horizontal)
        .constraints([Constraint::Percentage(100), Constraint::Percentage(50)].as_ref())
        .split(chunks[0]);
    let load = if datapoint.get_load_onoff() > 0.0 {
        "On"
    } else {
        "Off"
    };
    let load_current = datapoint.get_load_current().to_string();
    let battery_voltage = datapoint.get_battery_voltage().to_string();
    let battery_temp = datapoint.get_battery_temp().to_string();
    let pv_voltage = datapoint.get_pv_voltage().to_string();
    let charging = if datapoint.get_charging() > 0.0 {
        "Yes"
    } else {
        "No"
    };
    let charge_current = datapoint.get_charge_current().to_string();
    let over_discharge = datapoint.get_over_discharge().to_string();
    let battery_max = datapoint.get_battery_max().to_string();
    let battery_full = if datapoint.get_battery_full() > 0.0 {
        "Yes"
    } else {
        "No"
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
}
