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
    const SERIAL_TIMEOUT: u64 = 1000;

    pub(crate) fn get_comms() -> Vec<String> {
        let ports = serialport::available_ports().expect("Error reading ports.");
        ports.into_iter().map(|x| x.port_name).collect()
    }

    pub(crate) fn new(port_name: String) -> Self {
        Self {
            database: Database::default(),
            port: serialport::new(port_name, Self::BAUD_RATE)
                .timeout(Duration::from_millis(Self::SERIAL_TIMEOUT))
                .open()
                .unwrap(),
        }
    }

    fn read_serial_datapoint(&mut self) -> Result<String, std::io::Error> {
        let mut buf = Vec::new();
        let mut temp_buf = [0u8; 1]; // Temporary buffer to read one byte at a time
        loop {
            let bytes_read = self.port.read(&mut temp_buf)?;
            if bytes_read == 0 {
                // No more bytes available to read
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
}
