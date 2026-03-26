//! Standalone helper binary that exposes the MPPT simulator through a Unix PTY.

#[cfg(unix)]
fn main() -> Result<(), Box<dyn std::error::Error>> {
    use std::{thread, time::Duration};

    let harness = rust_solar::PtyControllerHarness::spawn()?;
    let injected_port = format!("{}{}", rust_solar::PTY_PORT_PREFIX, harness.slave_path());

    println!("PTY MPPT simulator ready on {}", harness.slave_path());
    println!(
        "Run the app with: SOLAR_EXTRA_PORTS={} cargo run",
        injected_port
    );
    println!("Press Ctrl+C to stop the simulator.");

    loop {
        thread::sleep(Duration::from_secs(60));
    }
}

#[cfg(not(unix))]
fn main() {
    eprintln!("The PTY-backed simulator is only available on Unix-like systems.");
}
