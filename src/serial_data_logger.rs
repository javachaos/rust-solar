use crate::database::Database;
use crate::datapoint::DataPoint;
use serialport::SerialPort;
use std::io::Read;
use std::time::Duration;

pub(crate) struct SerialDatalogger {
    database: Database,
    port: Box<dyn SerialPort>,
}

impl SerialDatalogger {
    const BAUD_RATE: u32 = 57600;
    const SERIAL_TIMEOUT: u64 = 2000;

    pub(crate) fn get_comms() -> Vec<String> {
        let ports = serialport::available_ports().expect("Error reading ports.");
        ports.into_iter().map(|x| x.port_name).collect()
    }

    pub(crate) fn new(port_name: String) -> Self {
        Self {
            database: Database::default(),
            port: {
                match serialport::new(port_name, Self::BAUD_RATE)
                    .timeout(Duration::from_millis(Self::SERIAL_TIMEOUT))
                    .open()
                {
                    Ok(p) => p,
                    Err(e) => panic!("Error: {}", e),
                }
            },
        }
    }

    pub(crate) fn read_serial_datapoint(&mut self) -> Result<String, std::io::Error> {
        let mut buf = Vec::new();
        let mut temp_buf = [0u8; 1];
        loop {
            let bytes_read = self.port.read(&mut temp_buf)?;
            if bytes_read == 0 {
                break;
            }
            buf.push(temp_buf[0]);
            if temp_buf[0] == b'\n' {
                // Newline byte encountered, stop reading
                break;
            }
        }
        let data = String::from_utf8_lossy(&buf).to_string();
        Ok(data
            .trim_end_matches(|c| c == '\r' || c == '\n')
            .to_string())
    }

    pub(crate) fn read_datapoint(&mut self) -> DataPoint {
        match self.read_serial_datapoint() {
            Ok(data) => {
                let dp = DataPoint::from_str(data.as_str());
                self.database.add_datapoint(dp);
                dp
            }
            Err(e) => {
                error!("Error: {}.", e);
                DataPoint::default()
            }
        }
    }

    fn write(&mut self, data: &str) -> usize {
        let x = match self.port.write(data.as_bytes()) {
            Ok(p) => p,
            Err(e) => {
                error!("{}", e);
                0
            }
        };
        x
    }

    ///Toggle the load on or off
    pub(crate) fn load_on(&mut self) {
        let _ = self.read_serial_datapoint();
        let x = self.write("LON\n");
        info!("Wrote {} bytes over serial.", x);
        let _ = self.port.flush();
    }

    pub(crate) fn load_off(&mut self) {
        let _ = self.read_serial_datapoint();
        let x = self.write("LOFF\n");
        info!("Wrote {} bytes over serial.", x);
        let _ = self.port.flush();
    }
}
