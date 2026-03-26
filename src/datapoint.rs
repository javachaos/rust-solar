//! Data model and parser for MPPT controller telemetry.
//!
//! The controller emits colon-delimited frames with exactly 10 numeric fields in this order:
//!
//! 1. battery voltage
//! 2. PV voltage
//! 3. load current
//! 4. over-discharge threshold
//! 5. battery max voltage
//! 6. battery full flag
//! 7. charging flag
//! 8. battery temperature
//! 9. charge current
//! 10. load enabled flag

use std::{
    fmt,
    num::ParseFloatError,
    str::FromStr,
    time::{Duration, SystemTime, SystemTimeError, UNIX_EPOCH},
};

use chrono::{DateTime, Local};
use log::warn;

const FIELD_COUNT: usize = 10;

/// Parsed snapshot of one controller telemetry frame plus a local receive timestamp.
#[derive(Debug, Clone, Copy, PartialEq)]
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

/// Parse failure for an incoming controller frame.
#[derive(Debug)]
pub(crate) enum DataPointParseError {
    InvalidFieldCount {
        expected: usize,
        actual: usize,
    },
    InvalidNumber {
        index: usize,
        source: ParseFloatError,
    },
}

impl fmt::Display for DataPointParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidFieldCount { expected, actual } => {
                write!(
                    f,
                    "invalid datapoint field count: expected {expected}, got {actual}"
                )
            }
            Self::InvalidNumber { index, source } => {
                write!(f, "invalid datapoint value at index {index}: {source}")
            }
        }
    }
}

impl std::error::Error for DataPointParseError {}

impl fmt::Display for DataPoint {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
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
        Self::from_values([0.0; FIELD_COUNT])
    }
}

impl FromStr for DataPoint {
    type Err = DataPointParseError;

    fn from_str(data_str: &str) -> Result<Self, Self::Err> {
        let mut values = [0.0; FIELD_COUNT];
        let mut count = 0;

        for (index, raw_value) in data_str.trim().split(':').enumerate() {
            if index >= FIELD_COUNT {
                return Err(DataPointParseError::InvalidFieldCount {
                    expected: FIELD_COUNT,
                    actual: index + 1,
                });
            }

            values[index] = raw_value
                .parse::<f64>()
                .map_err(|source| DataPointParseError::InvalidNumber { index, source })?;
            count += 1;
        }

        if count != FIELD_COUNT {
            return Err(DataPointParseError::InvalidFieldCount {
                expected: FIELD_COUNT,
                actual: count,
            });
        }

        Ok(Self::from_values(values))
    }
}

impl DataPoint {
    /// Creates a datapoint from raw controller values using the current local timestamp.
    pub(crate) fn from_values(values: [f64; FIELD_COUNT]) -> Self {
        Self::from_values_with_timestamp(current_timestamp(), values)
    }

    /// Creates a datapoint from raw controller values and an explicit timestamp.
    pub(crate) fn from_values_with_timestamp(timestamp: i64, values: [f64; FIELD_COUNT]) -> Self {
        Self {
            timestamp,
            battery_voltage: values[0],
            pv_voltage: values[1],
            load_current: values[2],
            over_discharge: values[3],
            battery_max: values[4],
            battery_full: values[5],
            charging: values[6],
            battery_temp: values[7],
            charge_current: values[8],
            load_onoff: values[9],
        }
    }

    pub(crate) fn get_time(&self) -> i64 {
        self.timestamp
    }

    pub(crate) fn get_time_formatted(&self) -> String {
        DateTime::from_timestamp(self.timestamp, 0)
            .map(|date| date.with_timezone(&Local).to_rfc2822())
            .unwrap_or_else(|| self.timestamp.to_string())
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

    /// Returns `true` when the controller reports the battery as full.
    pub(crate) fn is_battery_full(&self) -> bool {
        self.battery_full >= 1.0
    }

    /// Returns `true` when the controller reports active charging.
    pub(crate) fn is_charging(&self) -> bool {
        self.charging >= 1.0
    }

    /// Returns `true` when the controller reports the load output as enabled.
    pub(crate) fn is_load_enabled(&self) -> bool {
        self.load_onoff >= 1.0
    }
}

/// Returns the current local UNIX timestamp used when a frame is received.
fn current_timestamp() -> i64 {
    timestamp_from_elapsed(SystemTime::now().duration_since(UNIX_EPOCH))
}

fn timestamp_from_elapsed(duration_since_epoch: Result<Duration, SystemTimeError>) -> i64 {
    match duration_since_epoch {
        Ok(duration) => timestamp_from_seconds(duration.as_secs()),
        Err(_) => {
            warn!("SystemTime is before UNIX EPOCH, falling back to 0.");
            0
        }
    }
}

fn timestamp_from_seconds(seconds: u64) -> i64 {
    i64::try_from(seconds).unwrap_or_else(|_| {
        warn!("Current timestamp overflowed i64, clamping to i64::MAX.");
        i64::MAX
    })
}

#[cfg(test)]
mod tests {
    use std::time::{Duration, UNIX_EPOCH};

    use super::{timestamp_from_elapsed, timestamp_from_seconds, DataPoint, DataPointParseError};

    #[test]
    fn parses_valid_datapoint_payload() {
        let datapoint: DataPoint = "12.5:8.4:4.2:10.8:14.9:1:1:22:3.8:0"
            .parse()
            .expect("valid payload should parse");

        assert_eq!(datapoint.get_battery_voltage(), 12.5);
        assert_eq!(datapoint.get_pv_voltage(), 8.4);
        assert_eq!(datapoint.get_charge_current(), 3.8);
        assert!(!datapoint.is_load_enabled());
        assert!(datapoint.is_battery_full());
        assert!(datapoint.is_charging());
    }

    #[test]
    fn rejects_payloads_with_too_few_fields() {
        let err = "1:2:3"
            .parse::<DataPoint>()
            .expect_err("payload should fail");

        assert!(matches!(
            err,
            DataPointParseError::InvalidFieldCount {
                expected: 10,
                actual: 3
            }
        ));
    }

    #[test]
    fn rejects_payloads_with_too_many_fields() {
        let err = "1:2:3:4:5:6:7:8:9:10:11"
            .parse::<DataPoint>()
            .expect_err("payload should fail");

        assert!(matches!(
            err,
            DataPointParseError::InvalidFieldCount {
                expected: 10,
                actual: 11
            }
        ));
    }

    #[test]
    fn rejects_payloads_with_invalid_numbers() {
        let err = "1:2:3:4:5:6:7:8:not-a-number:10"
            .parse::<DataPoint>()
            .expect_err("payload should fail");

        assert!(matches!(
            err,
            DataPointParseError::InvalidNumber { index: 8, .. }
        ));
    }

    #[test]
    fn formats_invalid_timestamps_without_panicking() {
        let datapoint = DataPoint::from_values_with_timestamp(i64::MAX, [0.0; 10]);

        assert_eq!(datapoint.get_time_formatted(), i64::MAX.to_string());
    }

    #[test]
    fn parse_errors_have_human_readable_messages() {
        let count_message = DataPointParseError::InvalidFieldCount {
            expected: 10,
            actual: 3,
        }
        .to_string();
        let number_message = "1:2:3:4:5:6:7:8:not-a-number:10"
            .parse::<DataPoint>()
            .expect_err("payload should fail")
            .to_string();

        assert_eq!(
            count_message,
            "invalid datapoint field count: expected 10, got 3"
        );
        assert!(number_message.contains("invalid datapoint value at index 8"));
    }

    #[test]
    fn default_datapoint_uses_zero_values() {
        let datapoint = DataPoint::default();

        assert_eq!(datapoint.get_battery_voltage(), 0.0);
        assert_eq!(datapoint.get_pv_voltage(), 0.0);
        assert!(!datapoint.is_battery_full());
        assert!(!datapoint.is_charging());
        assert!(!datapoint.is_load_enabled());
    }

    #[test]
    fn display_includes_key_measurements() {
        let datapoint = DataPoint::from_values_with_timestamp(
            123,
            [1.0, 2.0, 3.0, 4.0, 5.0, 1.0, 0.0, 8.0, 9.0, 1.0],
        );
        let rendered = datapoint.to_string();

        assert!(rendered.contains("123"));
        assert!(rendered.contains("1v"));
        assert!(rendered.contains("9A"));
    }

    #[test]
    fn timestamp_from_elapsed_returns_zero_before_epoch() {
        let before_epoch = (UNIX_EPOCH - Duration::from_secs(1))
            .duration_since(UNIX_EPOCH)
            .expect_err("reversed timestamps should fail");

        assert_eq!(timestamp_from_elapsed(Err(before_epoch)), 0);
    }

    #[test]
    fn timestamp_from_seconds_clamps_overflow() {
        assert_eq!(timestamp_from_seconds(u64::MAX), i64::MAX);
    }
}
