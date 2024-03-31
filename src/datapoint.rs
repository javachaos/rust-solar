use std::fmt;
use std::fmt::Formatter;
use std::time::{SystemTime, UNIX_EPOCH};

use chrono::DateTime;
use regex::Regex;

const DATA_POINT_REGEX: &str = r"(([+-]?(\d*[.])?\d+):){9}(\d{1,19})";

#[derive(Debug, Clone, Copy)]
pub(crate) struct DataPoint {
    timestamp: i64,
    battery_voltage: f64,
    pv_voltage: f64,
    load_current: f64,
    over_discharge: f64,
    battery_max: f64,
    battery_full: f64,
    charging: f64,
    battery_temp: f64,
    charge_current: f64,
    load_onoff: f64,
}

impl fmt::Display for DataPoint {
    fn fmt(&self, f: &mut Formatter<'_>) -> fmt::Result {
        write!(
            f,
            "({}, {}v, {}v, {}A, {}, {}v, {}, {}, {}C, {}A, {})",
            self.timestamp,
            self.battery_voltage,
            self.pv_voltage,
            self.load_current,
            self.over_discharge,
            self.battery_max,
            self.battery_full,
            self.charging,
            self.battery_temp,
            self.charge_current,
            self.load_onoff
        )
    }
}

impl Default for DataPoint {
    fn default() -> Self {
        let now = SystemTime::now();
        let mut timestamp: i64 = 0;
        if let Ok(n) = now.duration_since(UNIX_EPOCH) {
            timestamp = n
                .as_secs()
                .try_into()
                .expect("Unable to convert u64 to i64");
        } else {
            error!("WARNING: SystemTime is before UNIX EPOCH!");
        }
        Self {
            timestamp,
            battery_voltage: 0.0,
            pv_voltage: 0.0,
            load_current: 0.0,
            over_discharge: 0.0,
            battery_max: 0.0,
            battery_full: 0.0,
            charging: 0.0,
            battery_temp: 0.0,
            charge_current: 0.0,
            load_onoff: 0.0,
        }
    }
}

impl DataPoint {
    pub(crate) fn new(data: &[f64]) -> Self {
        let now = SystemTime::now();
        let mut timestamp: i64 = 0;
        if let Ok(n) = now.duration_since(UNIX_EPOCH) {
            timestamp = n
                .as_secs()
                .try_into()
                .expect("Unable to convert u64 to i64");
        } else {
            error!("WARNING: SystemTime is before UNIX EPOCH!");
        }
        Self {
            timestamp,
            battery_voltage: data[0],
            pv_voltage: data[1],
            load_current: data[2],
            over_discharge: data[3],
            battery_max: data[4],
            battery_full: data[5],
            charging: data[6],
            battery_temp: data[7],
            charge_current: data[8],
            load_onoff: data[9],
        }
    }

    pub(crate) fn from_str(data_str: &str) -> Self {
        let regx = Regex::new(DATA_POINT_REGEX).unwrap();
        let Some(_caps) = regx.captures(data_str) else {
            panic!("Invalid DataPoint syntax.")
        };
        let data = data_str
            .split(':')
            .filter_map(|s| s.parse::<f64>().ok())
            .collect::<Vec<_>>();
        Self::new(&data)
    }

    pub(crate) fn get_time(&self) -> i64 {
        self.timestamp
    }

    pub(crate) fn get_time_formatted(&self) -> String {
        let date = DateTime::from_timestamp(self.timestamp, 0).unwrap();
        date.to_rfc2822()
    }

    pub(crate) fn get_battery_voltage(&self) -> f64 {
        self.battery_voltage
    }

    pub(crate) fn get_pv_voltage(&self) -> f64 {
        self.pv_voltage
    }

    pub(crate) fn get_load_current(&self) -> f64 {
        self.load_current
    }

    pub(crate) fn get_over_discharge(&self) -> f64 {
        self.over_discharge
    }

    pub(crate) fn get_battery_max(&self) -> f64 {
        self.battery_max
    }

    pub(crate) fn get_battery_full(&self) -> f64 {
        self.battery_full
    }

    pub(crate) fn get_charging(&self) -> f64 {
        self.charging
    }

    pub(crate) fn get_battery_temp(&self) -> f64 {
        self.battery_temp
    }

    pub(crate) fn get_charge_current(&self) -> f64 {
        self.charge_current
    }

    pub(crate) fn get_load_onoff(&self) -> f64 {
        self.load_onoff
    }
}
