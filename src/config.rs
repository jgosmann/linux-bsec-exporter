use std::collections::HashMap;

use bsec::{OutputKind, SampleRate, SubscriptionRequest};
use serde::{de::Error, Deserialize, Deserializer};

#[derive(Clone, Debug, Deserialize)]
pub struct Config {
    pub sensor: SensorConfig,

    #[serde(default)]
    pub bsec: BsecConfig,

    #[serde(default)]
    pub exporter: ExporterConfig,
}

#[derive(Clone, Debug, Deserialize)]
pub struct SensorConfig {
    pub device: String,

    #[serde(with = "I2CAddressDef")]
    #[serde(default)]
    pub address: bme680::I2CAddress,

    #[serde(default = "default_initial_ambient_temp_celsius")]
    pub initial_ambient_temp_celsius: f32,
}

fn default_initial_ambient_temp_celsius() -> f32 {
    20.0
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct BsecConfig {
    #[serde(default = "default_bsec_config")]
    pub config: String,

    #[serde(default)]
    pub temperature_offset_celsius: f32,

    #[serde(default = "default_bsec_state_file")]
    pub state_file: String,

    #[serde(deserialize_with = "deserialize_subscriptions")]
    #[serde(default = "all_bsec_subscriptions_config")]
    pub subscriptions: Vec<SubscriptionRequest>,
}

fn deserialize_subscriptions<'de, D>(deserializer: D) -> Result<Vec<SubscriptionRequest>, D::Error>
where
    D: Deserializer<'de>,
{
    let map = HashMap::<String, SampleRateDef>::deserialize(deserializer)?;
    map.iter()
        .map(|(k, v)| {
            Ok(SubscriptionRequest {
                sensor: output_kind_from_str::<D>(k)?,
                sample_rate: v.into(),
            })
        })
        .collect()
}

fn output_kind_from_str<'de, D>(variant: &str) -> Result<OutputKind, D::Error>
where
    D: Deserializer<'de>,
{
    use OutputKind::*;
    match variant {
        "iaq" => Ok(Iaq),
        "static_iaq" => Ok(StaticIaq),
        "co2_equivalent" => Ok(Co2Equivalent),
        "breath_voc_equivalent" => Ok(BreathVocEquivalent),
        "raw_temperature" => Ok(RawTemperature),
        "raw_pressure" => Ok(RawPressure),
        "raw_humidity" => Ok(RawHumidity),
        "raw_gas" => Ok(RawGas),
        "stabilization_status" => Ok(StabilizationStatus),
        "run_in_status" => Ok(RunInStatus),
        "sensor_heat_compensated_temperature" => Ok(SensorHeatCompensatedTemperature),
        "sensor_heat_compensated_humidity" => Ok(SensorHeatCompensatedHumidity),
        "gas_percentage" => Ok(GasPercentage),
        _ => Err(D::Error::unknown_variant(
            variant,
            &[
                "iaq",
                "static_iaq",
                "co2_equivalent",
                "breath_voc_equivalent",
                "raw_temperature",
                "raw_pressure",
                "raw_humidity",
                "raw_gas",
                "stablization_status",
                "run_in_status",
                "sensor_heat_compensated_temperature",
                "sensor_heat_compensated_humidity",
                "debug_compensated_gas",
                "gas_percentage",
            ],
        )),
    }
}

impl Default for BsecConfig {
    fn default() -> Self {
        Self {
            config: default_bsec_config(),
            temperature_offset_celsius: 0.,
            state_file: default_bsec_state_file(),
            subscriptions: all_bsec_subscriptions_config(),
        }
    }
}

fn default_bsec_config() -> String {
    "/etc/linux-bsec-exporter/bsec.conf".into()
}

fn default_bsec_state_file() -> String {
    "/var/lib/linux-bsec-exporter/bsec-state.bin".into()
}

fn all_bsec_subscriptions_config() -> Vec<SubscriptionRequest> {
    [
        OutputKind::Co2Equivalent,
        OutputKind::BreathVocEquivalent,
        OutputKind::RawTemperature,
        OutputKind::RawPressure,
        OutputKind::RawHumidity,
        OutputKind::RawGas,
        OutputKind::StabilizationStatus,
        OutputKind::RunInStatus,
        OutputKind::SensorHeatCompensatedTemperature,
        OutputKind::SensorHeatCompensatedHumidity,
        OutputKind::GasPercentage,
    ]
    .iter()
    .cloned()
    .map(|sensor| SubscriptionRequest {
        sensor,
        sample_rate: SampleRate::Lp,
    })
    .collect()
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
pub struct ExporterConfig {
    #[serde(default = "default_listen_addrs")]
    pub listen_addrs: Vec<String>,
}

impl Default for ExporterConfig {
    fn default() -> Self {
        Self {
            listen_addrs: default_listen_addrs(),
        }
    }
}

fn default_listen_addrs() -> Vec<String> {
    vec!["localhost:3953".into()]
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
#[serde(remote = "bme680::I2CAddress")]
enum I2CAddressDef {
    Primary,
    Secondary,
}

#[derive(Clone, Debug, Deserialize, PartialEq)]
#[serde(rename_all = "lowercase")]
pub enum SampleRateDef {
    Disabled,
    Ulp,
    Continuous,
    Lp,
}

impl Into<bsec::SampleRate> for &SampleRateDef {
    fn into(self) -> bsec::SampleRate {
        use SampleRateDef::*;
        match self {
            Disabled => bsec::SampleRate::Disabled,
            Ulp => bsec::SampleRate::Ulp,
            Continuous => bsec::SampleRate::Continuous,
            Lp => bsec::SampleRate::Lp,
        }
    }
}

#[cfg(test)]
pub mod tests {
    use std::collections::HashSet;

    use super::*;

    static FULL_CONFIG: &str = r#"
        [sensor]
        device = "/dev/i2c-1"
        address = "secondary"
        initial_ambient_temp_celsius = 25

        [bsec]
        config = "/etc/linux-bsec-exporter/bsec.conf"
        temperature_offset_celsius = 10.0
        state_file = "/var/lib/linux-bsec-exporter/bsec-state.bin"

        [bsec.subscriptions]
        iaq = "ulp"
        static_iaq = "ulp"
        co2_equivalent = "ulp"
        breath_voc_equivalent = "ulp"
        raw_temperature = "ulp"
        raw_pressure = "ulp"
        raw_humidity = "ulp"
        raw_gas = "ulp"
        stabilization_status = "ulp"
        run_in_status = "ulp"
        sensor_heat_compensated_temperature = "ulp"
        sensor_heat_compensated_humidity = "ulp"
        gas_percentage = "ulp"

        [exporter]
        listen_addrs = ["192.168.0.1:1234"]
    "#;

    static MINIMAL_CONFIG: &str = r#"
        [sensor]
        device = "/dev/i2c-1"
    "#;

    static MINIMAL_CONFIG_WITH_SECTION_HEADERS: &str = r#"
        [sensor]
        device = "/dev/i2c-1"
            
        [bsec]

        [exporter]
    "#;

    #[test]
    fn test_reading_full_toml_config() {
        let config: Config = toml::from_str(FULL_CONFIG).unwrap();

        assert_eq!(config.sensor.device, "/dev/i2c-1");
        if let bme680::I2CAddress::Secondary = config.sensor.address {
        } else {
            panic!(
                "Expected sensor.device.address to be {:?}, but got {:?}.",
                bme680::I2CAddress::Secondary,
                config.sensor.address
            );
        }
        assert_eq!(config.sensor.initial_ambient_temp_celsius, 25.);
        assert_eq!(
            config.exporter,
            ExporterConfig {
                listen_addrs: vec!["192.168.0.1:1234".into()]
            }
        );
        assert_eq!(
            config.bsec.config,
            String::from("/etc/linux-bsec-exporter/bsec.conf")
        );
        assert_eq!(config.bsec.temperature_offset_celsius, 10.);
        assert_eq!(
            config.bsec.state_file,
            String::from("/var/lib/linux-bsec-exporter/bsec-state.bin")
        );

        let subscriptions: HashSet<_> = config.bsec.subscriptions.into_iter().collect();
        let expected_subscriptions: HashSet<_> = [
            OutputKind::Iaq,
            OutputKind::StaticIaq,
            OutputKind::Co2Equivalent,
            OutputKind::BreathVocEquivalent,
            OutputKind::RawTemperature,
            OutputKind::RawPressure,
            OutputKind::RawHumidity,
            OutputKind::RawGas,
            OutputKind::StabilizationStatus,
            OutputKind::RunInStatus,
            OutputKind::SensorHeatCompensatedTemperature,
            OutputKind::SensorHeatCompensatedHumidity,
            OutputKind::GasPercentage,
        ]
        .iter()
        .map(|&sensor| SubscriptionRequest {
            sensor,
            sample_rate: SampleRate::Ulp,
        })
        .collect();
        assert_eq!(subscriptions, expected_subscriptions);
    }

    #[test]
    fn test_config_defaults() {
        let config: Config = toml::from_str(MINIMAL_CONFIG).unwrap();
        assert_config_defaults(config);
    }

    #[test]
    fn test_config_defaults_with_section_headers() {
        let config: Config = toml::from_str(MINIMAL_CONFIG_WITH_SECTION_HEADERS).unwrap();
        assert_config_defaults(config);
    }

    fn assert_config_defaults(config: Config) {
        assert_eq!(config.sensor.device, "/dev/i2c-1");
        if let bme680::I2CAddress::Primary = config.sensor.address {
        } else {
            panic!(
                "Expected sensor.device.address to be {:?}, but got {:?}.",
                bme680::I2CAddress::Primary,
                config.sensor.address
            );
        }
        assert_eq!(config.sensor.initial_ambient_temp_celsius, 20.);
        assert_eq!(
            config.exporter,
            ExporterConfig {
                listen_addrs: vec!["localhost:3953".into()]
            }
        );
        assert_eq!(
            config.bsec,
            BsecConfig {
                config: "/etc/linux-bsec-exporter/bsec.conf".into(),
                temperature_offset_celsius: 0.,
                state_file: "/var/lib/linux-bsec-exporter/bsec-state.bin".into(),
                subscriptions: all_bsec_subscriptions_config()
            }
        );
    }
}
