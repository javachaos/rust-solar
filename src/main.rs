mod database;
mod datapoint;
mod serial_data_logger;

#[macro_use] extern crate log;
extern crate simplelog;
use simplelog::*;
use std::{fs::File, process::exit};

use datapoint::DataPoint;
use serial_data_logger::SerialDatalogger;

use crossterm::{
    event::{self, DisableMouseCapture, EnableMouseCapture, Event, KeyCode},
    execute,
    terminal::{disable_raw_mode, enable_raw_mode, EnterAlternateScreen, LeaveAlternateScreen},
};
use std::{error::Error, io::{self, Write}, sync::{atomic::{AtomicBool, Ordering}, mpsc, Arc}, thread::{self, sleep}, time::Duration};
use tui::{
    backend::{Backend, CrosstermBackend},
    layout::{Alignment, Constraint, Direction, Layout},
    style::{Color, Style},
    widgets::{Block, BorderType, Borders, Cell, Row, Table},
    Frame, Terminal,
};

fn main() -> Result<(), Box<dyn Error>> {
    CombinedLogger::init(
        vec![
            TermLogger::new(LevelFilter::Warn, Config::default(), TerminalMode::Mixed, ColorChoice::Auto),
            WriteLogger::new(LevelFilter::Info, Config::default(), File::create("solar-rust.log").unwrap()),
        ]
    ).unwrap();
    let ports = SerialDatalogger::get_comms();
    let mut i = 0;
    for p in &ports {
        println!("{i}: {:?}", p);
        info!("{i}: {:?}", p);
        i+=1;
    }
    print!("Please select a port: ");
    info!("Please select a port: ");
    let mut port_index_str = String::new();

    let _ = io::stdout().flush();
    let mut handle = io::stdin().lock();
    match io::BufRead::read_line(&mut handle, &mut port_index_str) {
        
        Ok(d) => {
            #[cfg(debug_assertions)]
            println!("Read {} bytes.", d);
            info!("Read {} bytes: {}", d, port_index_str.escape_default())
        },
        Err(e) => error!("{}", e),
    }
    
    let port_index = port_index_str.trim().parse::<u32>().unwrap() as usize;
    if port_index > ports.len() {
        error!("Invalid port index.");
        sleep(Duration::from_secs(5));
        exit(0);
    }
    
    let selected_port = &ports[port_index];

    // setup terminal
    enable_raw_mode()?;
    let mut stdout = io::stdout();
    execute!(stdout, EnterAlternateScreen, EnableMouseCapture)?;
    let backend = CrosstermBackend::new(stdout);
    let mut terminal = Terminal::new(backend)?;

    let res = run_app(&mut terminal, selected_port);

    // restore terminal
    disable_raw_mode()?;
    execute!(
        terminal.backend_mut(),
        LeaveAlternateScreen,
        DisableMouseCapture
    )?;
    terminal.show_cursor()?;

    if let Err(err) = res {
        error!("{:?}", err)
    }

    Ok(())
}

fn run_app<B: Backend>(terminal: &mut Terminal<B>, selected_port: &String) -> io::Result<()> {
    let (rx, tx) = mpsc::channel();
    let mut data_logger = SerialDatalogger::new(selected_port.to_string());
    let running = Arc::new(AtomicBool::new(true));
    let builder = thread::Builder::new()
        .name("datalogger".into())
        .stack_size(1024 * 1024);//1MB
    let task = {
        let running = Arc::clone(&running);
        move || {
            while running.load(Ordering::SeqCst)  {
                let datapoint = data_logger.read_datapoint();
                rx.send(datapoint).unwrap();
                sleep(Duration::from_secs(1));
            }
        }
    };
    let handle = builder.spawn(task)
        .expect("Error: creating data logging thread failed.");
    let mut current_dp = DataPoint::default();
    loop {
        current_dp = match tx.recv_timeout(Duration::from_micros(1000)) {
            Ok(v) => v,
            Err(_e) => {
                current_dp 
            },
        };
        terminal.draw(|f| ui(f, current_dp))?;
        if let Event::Key(key) = event::read()? {
            if let KeyCode::Char('q') = key.code {
                running.store(false, Ordering::SeqCst);
                handle.join().unwrap();
                return Ok(());
            }
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
    let load = if datapoint.get_load_onoff() > 0.0 { "On" } else { "Off" };
    let load_current = datapoint.get_load_current().to_string();
    let battery_voltage = datapoint.get_battery_voltage().to_string();
    let battery_temp = datapoint.get_battery_temp().to_string();
    let pv_voltage = datapoint.get_pv_voltage().to_string();
    let charging = if datapoint.get_charging() > 0.0 { "Yes" } else { "No" };
    let charge_current = datapoint.get_charge_current().to_string();
    let over_discharge = datapoint.get_over_discharge().to_string();
    let battery_max = datapoint.get_battery_max().to_string();
    let battery_full = if datapoint.get_battery_full() > 0.0 { "Yes" } else { "No" };
    let table = Table::new(vec![
        Row::new(vec![Cell::from("Load: ").style(Style::default().fg(Color::Green)), Cell::from(load)]),
        Row::new(vec![Cell::from("Load Current: ").style(Style::default().fg(Color::Green)), Cell::from(load_current)]),
        Row::new(vec![Cell::from("Battery Voltage: ").style(Style::default().fg(Color::Green)), Cell::from(battery_voltage)]),
        Row::new(vec![Cell::from("Battery Full: ").style(Style::default().fg(Color::Green)), Cell::from(battery_full)]),
        Row::new(vec![Cell::from("Battery Temp: ").style(Style::default().fg(Color::Green)), Cell::from(battery_temp)]),
        Row::new(vec![Cell::from("PV Voltage: ").style(Style::default().fg(Color::Green)), Cell::from(pv_voltage)]),
        Row::new(vec![Cell::from("Charging: ").style(Style::default().fg(Color::Green)), Cell::from(charging)]),
        Row::new(vec![Cell::from("Charge Current: ").style(Style::default().fg(Color::Green)), Cell::from(charge_current)]),
        Row::new(vec![Cell::from("Over Discharge: ").style(Style::default().fg(Color::Green)), Cell::from(over_discharge)]),
        Row::new(vec![Cell::from("Battery Max: ").style(Style::default().fg(Color::Green)), Cell::from(battery_max)]),
    ])
    .style(Style::default().fg(Color::White))
    .block(Block::default().title("MPPT Data"))
    .widths(&[Constraint::Length(25), Constraint::Length(5), Constraint::Length(10)])
    .column_spacing(1);
    f.render_widget(table, top_chunks[0]);
}