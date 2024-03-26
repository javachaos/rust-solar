mod database;
mod datapoint;
mod serial_data_logger;

#[macro_use]
extern crate log;
extern crate simplelog;
use simplelog::{
    ColorChoice, CombinedLogger, Config, LevelFilter, TermLogger, TerminalMode, WriteLogger,
};

use datapoint::DataPoint;
use serial_data_logger::SerialDatalogger;

use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode, MouseEventKind},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use std::{
    collections::VecDeque,
    error::Error,
    fs::File,
    io,
    sync::{
        atomic::{AtomicBool, Ordering},
        mpsc, Arc,
    },
    thread::{self, sleep},
    time::{Duration, Instant},
};
use tui::{
    backend::{Backend, CrosstermBackend},
    buffer::Buffer,
    layout::{Alignment, Constraint, Direction, Layout, Rect},
    style::{Color, Modifier, Style},
    symbols::{self, Marker},
    text::{Span, Spans},
    widgets::{
        Axis, Block, BorderType, Borders, Cell, Chart, Dataset, GraphType, List, ListItem,
        ListState, Row, Table,
    },
    Frame, Terminal,
};

// These types were too long...
type TermType = Terminal<CrosstermBackend<std::io::Stdout>>;
type TermResult = Result<Terminal<CrosstermBackend<std::io::Stdout>>, Box<dyn Error>>;

const LOGFILE_PATH: &str = "solar-rust.log";

/// A custom widget for a toggle switch.

#[derive(Debug, Clone, Copy)]
struct LoadToggleSwitch<'a> {
    is_on: bool,
    labels: (&'a str, &'a str),
}

impl<'a> LoadToggleSwitch<'a> {
    pub fn new(is_on: bool, labels: (&'a str, &'a str)) -> LoadToggleSwitch<'a> {
        LoadToggleSwitch { is_on, labels }
    }
}

impl<'a> tui::widgets::Widget for LoadToggleSwitch<'a> {
    fn render(self, area: Rect, buf: &mut Buffer) {
        let on_label = Span::styled(
            self.labels.0,
            Style::default().fg(if self.is_on {
                Color::Green
            } else {
                Color::DarkGray
            }),
        );
        let off_label = Span::styled(
            self.labels.1,
            Style::default().fg(if !self.is_on {
                Color::Red
            } else {
                Color::DarkGray
            }),
        );

        let switch = if self.is_on {
            Span::styled(
                symbols::line::VERTICAL,
                Style::default().add_modifier(Modifier::BOLD),
            )
        } else {
            Span::raw(" ")
        };

        let spans = Spans::from(vec![on_label, switch, off_label]);
        let block = Block::default().borders(Borders::ALL).title("Load");
        let inner_area = block.inner(area);
        block.render(area, buf);
        buf.set_spans(inner_area.x, inner_area.y, &spans, inner_area.width);
    }
}

fn main() -> Result<(), Box<dyn Error>> {
    setup_logging()?;

    let ports = SerialDatalogger::get_comms();
    let mut terminal = setup_terminal()?;

    let mut port_list_state = ListState::default();
    port_list_state.select(Some(0));
    let _ = display_ports(&mut terminal, &ports, &mut port_list_state);
    let selected_port = &ports[port_list_state.selected().unwrap()];

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

fn display_ports<B: Backend>(
    terminal: &mut Terminal<B>,
    ports: &[String],
    port_list_state: &mut ListState,
) -> io::Result<()> {
    for (i, p) in ports.iter().enumerate() {
        info!("{i}: {p:?}");
    }

    loop {
        let _ = terminal.draw(|f| init_ui(f, ports.to_vec(), port_list_state));
        if crossterm::event::poll(Duration::from_micros(100))? {
            if let Event::Key(key) = event::read()? {
                if let KeyCode::Enter = key.code {
                    return Ok(());
                }
                if let KeyCode::Up = key.code {
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

fn run_app<B: Backend>(
    terminal: &mut Terminal<B>,
    selected_port: &String,
    tick_rate: Duration,
) -> io::Result<()> {
    let mut last_tick = Instant::now();
    let (rx, tx) = mpsc::channel();
    let (bg_tx, bg_rx) = mpsc::channel();
    let mut data_logger = SerialDatalogger::new(selected_port.to_string());
    let initial_dp = data_logger.read_datapoint();
    let mut load_switch = LoadToggleSwitch::new(initial_dp.get_load_onoff() > 0.0, ("ON", "OFF"));
    let toggle = Arc::new(AtomicBool::new(load_switch.is_on));
    let running = Arc::new(AtomicBool::new(true));
    let builder = thread::Builder::new()
        .name("datalogger".into())
        .stack_size(1024 * 1024); //1MB
    let task = {
        let running = Arc::clone(&running);
        let toggle = Arc::clone(&toggle);
        move || {
            while running.load(Ordering::SeqCst) {
                let datapoint = data_logger.read_datapoint();
                rx.send(datapoint).unwrap();
                sleep(Duration::from_secs(1));
                match bg_rx.recv_timeout(Duration::from_micros(1000)) {
                    Ok(_) => {
                        if toggle.load(Ordering::SeqCst) {
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
    let handle = builder
        .spawn(task)
        .expect("Error: creating data logging thread failed.");
    let mut current_dp = DataPoint::default();
    let mut data_buffer = Vec::with_capacity(256);
    loop {
        current_dp = match tx.recv_timeout(Duration::from_micros(1000)) {
            Ok(v) => {
                data_buffer.push(v);
                v
            }
            Err(_e) => current_dp,
        };
        terminal.draw(|f| ui(f, current_dp, data_buffer.clone().into(), load_switch))?;

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
            if let Event::Mouse(mouse_event) = event::read()? {
                if let MouseEventKind::Down(_) = mouse_event.kind {
                    info!("Mouse down event.");
                    if mouse_event.row == 1 && mouse_event.column <= 10 {
                        if load_switch.is_on {
                            toggle.store(false, Ordering::SeqCst);
                            load_switch.is_on = false;
                        } else {
                            toggle.store(true, Ordering::SeqCst);
                            load_switch.is_on = true;
                        }
                        bg_tx.send(DataPoint::default()).unwrap();
                    }
                }
            }
        }
        if last_tick.elapsed() >= tick_rate {
            last_tick = Instant::now();
        }
    }
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
                .title("Port Selection")
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
    data_buffer: VecDeque<DataPoint>,
    load_switch: LoadToggleSwitch,
) {
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
        .constraints([Constraint::Percentage(50), Constraint::Percentage(50)].as_ref())
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

    // Create the X axis and define its properties
    let x_axis = Axis::default()
        .title("Time (s)")
        .style(Style::default())
        .bounds([0.0, 256.0])
        .labels(vec!["0.0".into(), "128.0".into(), "256.0".into()]);

    // Create the Y axis and define its properties
    let y_axis = Axis::default()
        .title("Load Current (A)")
        .style(Style::default())
        .bounds([0.0, 100.0])
        .labels(vec!["0.0".into(), "50.0".into(), "100.0".into()]);

    let load_current_buffer = data_buffer
        .iter()
        .enumerate()
        .map(|(i, f)| (i as f64, f.get_load_current()))
        .collect::<Vec<(f64, f64)>>();
    let chart = Chart::new(vec![Dataset::default()
        .marker(Marker::Block)
        .graph_type(GraphType::Scatter)
        .data(load_current_buffer.as_slice())])
    .block(Block::default().title("Load Current vs Time"))
    .x_axis(x_axis)
    .y_axis(y_axis);
    f.render_widget(chart, top_chunks[1]);
    let area = Rect::new(size.x, size.y, 10, 2);
    f.render_widget(load_switch, area);
}
