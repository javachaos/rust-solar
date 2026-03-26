//! Buffered SQLite persistence for controller datapoints.
//!
//! The runtime writes datapoints frequently, so this module batches inserts in memory and
//! commits them in a single transaction once the buffer reaches a threshold or the app exits.

use std::{mem, path::Path};

use log::{info, warn};
use rusqlite::{params, Connection};

use crate::datapoint::DataPoint;

#[cfg_attr(test, allow(dead_code))]
const DATABASE_FILENAME: &str = "solar_data.sql";
const BUFFER_LIMIT: usize = 256;
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

/// SQLite-backed datapoint store with buffered writes.
pub(crate) struct Database {
    connection: Connection,
    datapoint_buffer: Vec<DataPoint>,
    buffer_limit: usize,
}

impl Database {
    #[cfg_attr(test, allow(dead_code))]
    /// Opens the default on-disk database file in the current working directory.
    pub(crate) fn open_default() -> rusqlite::Result<Self> {
        Self::open_path(DATABASE_FILENAME)
    }

    /// Opens or creates a database at the provided path.
    pub(crate) fn open_path<P: AsRef<Path>>(path: P) -> rusqlite::Result<Self> {
        Self::with_connection(Connection::open(path)?, BUFFER_LIMIT)
    }

    /// Adds a datapoint to the in-memory buffer and flushes automatically when full.
    pub(crate) fn add_datapoint(&mut self, datapoint: DataPoint) -> rusqlite::Result<()> {
        self.datapoint_buffer.push(datapoint);

        if self.datapoint_buffer.len() >= self.buffer_limit {
            self.flush()?;
        }

        Ok(())
    }

    /// Flushes the buffered datapoints inside a single SQLite transaction.
    pub(crate) fn flush(&mut self) -> rusqlite::Result<()> {
        if self.datapoint_buffer.is_empty() {
            return Ok(());
        }

        let datapoints = mem::replace(
            &mut self.datapoint_buffer,
            Vec::with_capacity(self.buffer_limit),
        );
        self.insert_datapoints(datapoints)
    }

    fn with_connection(connection: Connection, buffer_limit: usize) -> rusqlite::Result<Self> {
        connection.execute(DATABASE_CREATE_STMT, [])?;

        Ok(Self {
            connection,
            datapoint_buffer: Vec::with_capacity(buffer_limit),
            buffer_limit,
        })
    }

    fn insert_datapoints(&mut self, datapoints: Vec<DataPoint>) -> rusqlite::Result<()> {
        let datapoint_count = datapoints.len();
        let transaction = self.connection.transaction()?;

        {
            let mut statement = transaction.prepare_cached(DATABASE_INSERT)?;
            for datapoint in datapoints {
                statement.execute(params![
                    datapoint.get_battery_voltage(),
                    datapoint.get_pv_voltage(),
                    datapoint.get_load_current(),
                    datapoint.get_over_discharge(),
                    datapoint.get_battery_max(),
                    datapoint.get_battery_full(),
                    datapoint.get_charging(),
                    datapoint.get_battery_temp(),
                    datapoint.get_charge_current(),
                    datapoint.get_load_onoff(),
                    datapoint.get_time(),
                ])?;
            }
        }

        transaction.commit()?;
        info!("Wrote {datapoint_count} datapoints to database.");
        Ok(())
    }

    #[cfg(test)]
    fn open_in_memory_with_buffer_limit(buffer_limit: usize) -> rusqlite::Result<Self> {
        Self::with_connection(Connection::open_in_memory()?, buffer_limit)
    }

    #[cfg(test)]
    fn row_count(&self) -> rusqlite::Result<i64> {
        self.connection
            .query_row("SELECT COUNT(*) FROM Data", [], |row| row.get(0))
    }
}

impl Drop for Database {
    fn drop(&mut self) {
        log_drop_flush_result(self.flush());
    }
}

/// Logs a best-effort flush failure that occurs during `Drop`.
fn log_drop_flush_result(result: rusqlite::Result<()>) {
    if let Err(err) = result {
        warn!("Failed to flush database on drop: {err}");
    }
}

#[cfg(test)]
mod tests {
    use std::{
        env::{current_dir, set_current_dir, temp_dir},
        fs,
        path::PathBuf,
        sync::Mutex,
        time::{SystemTime, UNIX_EPOCH},
    };

    use rusqlite::Connection;

    use super::{log_drop_flush_result, Database};
    use crate::datapoint::DataPoint;

    static DATABASE_TEST_LOCK: Mutex<()> = Mutex::new(());

    fn sample_datapoint(load_onoff: f64) -> DataPoint {
        DataPoint::from_values_with_timestamp(
            1_710_000_000,
            [12.5, 18.2, 4.1, 10.7, 14.8, 1.0, 1.0, 24.0, 5.6, load_onoff],
        )
    }

    fn temp_database_path(name: &str) -> PathBuf {
        let unique = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .expect("current time should be after UNIX epoch")
            .as_nanos();
        temp_dir().join(format!("rust-solar-{name}-{unique}.sqlite"))
    }

    #[test]
    fn flush_persists_buffered_rows() {
        let mut database =
            Database::open_in_memory_with_buffer_limit(8).expect("database should open");

        database
            .add_datapoint(sample_datapoint(0.0))
            .expect("insert should succeed");
        database
            .add_datapoint(sample_datapoint(1.0))
            .expect("insert should succeed");

        assert_eq!(database.row_count().expect("count should succeed"), 0);

        database.flush().expect("flush should succeed");

        assert_eq!(database.row_count().expect("count should succeed"), 2);
    }

    #[test]
    fn reaching_buffer_limit_flushes_automatically() {
        let mut database =
            Database::open_in_memory_with_buffer_limit(2).expect("database should open");

        database
            .add_datapoint(sample_datapoint(0.0))
            .expect("insert should succeed");
        assert_eq!(database.row_count().expect("count should succeed"), 0);

        database
            .add_datapoint(sample_datapoint(1.0))
            .expect("insert should succeed");

        assert_eq!(database.row_count().expect("count should succeed"), 2);
    }

    #[test]
    fn open_default_creates_the_default_database_file() {
        let _guard = DATABASE_TEST_LOCK
            .lock()
            .expect("database test lock should be available");
        let temp_directory = temp_dir().join(format!(
            "rust-solar-default-db-{}",
            SystemTime::now()
                .duration_since(UNIX_EPOCH)
                .expect("current time should be after UNIX epoch")
                .as_nanos()
        ));
        fs::create_dir(&temp_directory).expect("temporary directory should be created");
        let original_directory = current_dir().expect("current directory should be available");

        set_current_dir(&temp_directory).expect("working directory should change");
        let mut database = Database::open_default().expect("default database should open");
        database
            .add_datapoint(sample_datapoint(1.0))
            .expect("insert should succeed");
        database.flush().expect("flush should succeed");

        assert!(temp_directory.join("solar_data.sql").exists());

        drop(database);
        set_current_dir(&original_directory).expect("working directory should be restored");
        fs::remove_file(temp_directory.join("solar_data.sql"))
            .expect("temporary database should be removable");
        fs::remove_dir(&temp_directory).expect("temporary directory should be removable");
    }

    #[test]
    fn drop_flushes_remaining_buffered_rows() {
        let path = temp_database_path("drop-flush");

        {
            let mut database = Database::open_path(&path).expect("database should open");
            database
                .add_datapoint(sample_datapoint(0.0))
                .expect("insert should succeed");
        }

        let connection = Connection::open(&path).expect("database should be reopenable");
        let row_count: i64 = connection
            .query_row("SELECT COUNT(*) FROM Data", [], |row| row.get(0))
            .expect("count should succeed");

        assert_eq!(row_count, 1);
        drop(connection);
        fs::remove_file(path).expect("temporary database should be removable");
    }

    #[test]
    fn drop_flush_logging_helper_accepts_errors() {
        log_drop_flush_result(Err(rusqlite::Error::InvalidQuery));
    }
}
