use crate::datapoint::DataPoint;
use rusqlite::Connection;
use std::mem;

//
// Constants
//
const DATABASE_FILENAME: &str = "solar_data.sql";
const BUFFER_LIMIT: usize = 256; //88 * 256 = ~22.5 kb (buffer size)
const DATABASE_CREATE_STMT: &str = concat!(
    "CREATE TABLE IF NOT EXISTS Data ",
    "(ID INTEGER PRIMARY KEY AUTOINCREMENT UNIQUE NOT",
    " NULL,battery_voltage DOUBLE, pv_voltage DOUBLE, load_current DOUBLE,",
    " over_discharge DOUBLE,battery_max DOUBLE, battery_full BOOLEAN, charging",
    " BOOLEAN, battery_temp DOUBLE,charge_current DOUBLE, load_onoff BOOLEAN, time",
    " TIMESTAMP DEFAULT CURRENT_TIMESTAMP)"
);
const DATABASE_INSERT: &str = concat!(
    "INSERT INTO Data(",
    "battery_voltage, ",
    "pv_voltage, ",
    "load_current, ",
    "over_discharge,",
    "battery_max, ",
    "battery_full, ",
    "charging, ",
    "battery_temp, ",
    "charge_current, ",
    "load_onoff,",
    "time",
    ") VALUES(?,?,?,?,?,?,?,?,?,?,?)"
);

//
// Structs
//
pub(crate) struct Database {
    connection: Connection,
    datapoint_buffer: Vec<DataPoint>,
}

//
// implementations
//
impl Default for Database {
    fn default() -> Self {
        let connection = Connection::open(DATABASE_FILENAME).unwrap();
        let _ = connection.execute(DATABASE_CREATE_STMT, ());
        Self {
            connection,
            datapoint_buffer: Vec::with_capacity(BUFFER_LIMIT),
        }
    }
}

impl Drop for Database {
    fn drop(&mut self) {
        let data = mem::take(&mut self.datapoint_buffer);
        self.insert_datapoints(data);
    }
}

impl Database {
    ///
    /// Add a datapoint to the internal buffer which will be added into
    /// the database when drop is called on this database object or
    /// when the buffer is filled.
    ///
    pub(crate) fn add_datapoint(&mut self, datapoint: DataPoint) {
        self.datapoint_buffer.push(datapoint);
        if self.datapoint_buffer.len() >= BUFFER_LIMIT {
            let data = mem::replace(&mut self.datapoint_buffer, Vec::with_capacity(BUFFER_LIMIT));
            self.insert_datapoints(data);
        }
    }

    ///
    /// Insert a vector of datapoints into the database in one atomic operation.
    ///
    fn insert_datapoints(&mut self, datapoints: Vec<DataPoint>) {
        let trans = self.connection.transaction().unwrap();
        let num_data = datapoints.len();
        for dp in datapoints {
            match trans.execute(
                DATABASE_INSERT,
                (
                    dp.get_battery_voltage(),
                    dp.get_pv_voltage(),
                    dp.get_load_current(),
                    dp.get_over_discharge(),
                    dp.get_battery_max(),
                    dp.get_battery_full(),
                    dp.get_charging(),
                    dp.get_battery_temp(),
                    dp.get_charge_current(),
                    dp.get_load_onoff(),
                    dp.get_time(),
                ),
            ) {
                Ok(_) => {}
                Err(e) => error!("{}", e),
            }
        }
        match trans.commit() {
            Ok(()) => {
                info!("Wrote {} datapoints to database.", num_data);
            }
            Err(e) => error!("{}", e),
        }
    }
}
