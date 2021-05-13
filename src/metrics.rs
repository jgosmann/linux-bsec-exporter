use std::{collections::HashMap, convert::TryFrom};

use prometheus::{proto::MetricFamily, Gauge, Opts, Registry};

struct GaugeUnit<'a> {
    ident_suffix: &'a str,
    display: &'a str,
}

impl<'a> GaugeUnit<'a> {
    fn new(unit: &'a str) -> Self {
        Self {
            ident_suffix: unit,
            display: unit,
        }
    }

    fn new_with_display(ident_suffix: &'a str, display: &'a str) -> Self {
        Self {
            ident_suffix,
            display,
        }
    }
}

#[derive(Clone)]
struct BsecGauge {
    value: Gauge,
    accuracy: Gauge,
}

impl BsecGauge {
    fn new(name: &str, help: &str, unit: Option<&GaugeUnit>) -> prometheus::Result<Self> {
        let value = if let Some(unit) = unit {
            Gauge::with_opts(Opts::new(
                format!("{}_{}", name, unit.ident_suffix),
                format!("{} ({})", help, unit.display),
            ))?
        } else {
            Gauge::with_opts(Opts::new(name, help))?
        };

        Ok(Self {
            value,
            accuracy: Gauge::with_opts(Opts::new(
                format!("{}_accuracy", name),
                format!("{} (accuracy)", help),
            ))?,
        })
    }

    fn register(&self, registry: &Registry) -> prometheus::Result<()> {
        registry.register(Box::new(self.value.clone()))?;
        registry.register(Box::new(self.accuracy.clone()))?;
        Ok(())
    }

    fn set(&self, value: f64, accuracy: bsec::Accuracy) {
        self.value.set(value);
        self.accuracy.set((accuracy as u8).into());
    }
}

impl TryFrom<&bsec::OutputKind> for BsecGauge {
    type Error = prometheus::Error;

    fn try_from(sensor: &bsec::OutputKind) -> Result<Self, Self::Error> {
        use bsec::OutputKind::*;
        match sensor {
            Iaq => BsecGauge::new("iaq", "Indoor-air-quality estimate [0-500]", None),
            StaticIaq => BsecGauge::new("static_iaq", "Unscaled indoor-air-quality estimate", None),
            Co2Equivalent => BsecGauge::new(
                "co2_equivalent",
                "CO2 equivalent estimate",
                Some(&GaugeUnit::new("ppm")),
            ),
            BreathVocEquivalent => BsecGauge::new(
                "breath_voc_equivalent",
                "Breath VOC concentration estimate",
                Some(&GaugeUnit::new("ppm")),
            ),
            RawTemperature => BsecGauge::new(
                "raw_temperature",
                "Temperature sensor signal",
                Some(&GaugeUnit::new_with_display("celsius", "°C")),
            ),
            RawPressure => BsecGauge::new(
                "raw_pressure",
                "Pressure sensor signal",
                Some(&GaugeUnit::new("Pa")),
            ),
            RawHumidity => BsecGauge::new(
                "raw_humidity",
                "Relative humidity sensor signal",
                Some(&GaugeUnit::new_with_display("percent", "%")),
            ),
            RawGas => BsecGauge::new(
                "raw_gas",
                "Gas sensor signal",
                Some(&GaugeUnit::new_with_display("ohm", "Ω")),
            ),
            StabilizationStatus => BsecGauge::new(
                "stabilization_status",
                "Gas sensor stabilization status (boolean)",
                None,
            ),
            RunInStatus => {
                BsecGauge::new("run_in_status", "Gas sensor run-in status (boolean)", None)
            }
            SensorHeatCompensatedTemperature => BsecGauge::new(
                "temperature",
                "Sensor heat compensated temperature",
                Some(&GaugeUnit::new_with_display("celsius", "°C")),
            ),
            SensorHeatCompensatedHumidity => BsecGauge::new(
                "humidity",
                "Sensor heat compensated humidity",
                Some(&GaugeUnit::new_with_display("percent", "%")),
            ),
            DebugCompensatedGas => BsecGauge::new(
                "debug_compensated_gas",
                "Reserved internal debug output",
                None,
            ),
            GasPercentage => BsecGauge::new(
                "gas",
                "Percentage of min and max filtered gas value",
                Some(&GaugeUnit::new_with_display("percent", "%")),
            ),
        }
    }
}

#[derive(Clone)]
pub struct BsecGaugeRegistry {
    registry: Registry,
    sensor_gauge_map: HashMap<bsec::OutputKind, BsecGauge>,
}

impl BsecGaugeRegistry {
    pub fn new(sensors: &[bsec::OutputKind]) -> prometheus::Result<Self> {
        let mut gauge_registry = Self {
            registry: Registry::new(),
            sensor_gauge_map: HashMap::with_capacity(sensors.len()),
        };

        for sensor in sensors {
            let gauge = BsecGauge::try_from(sensor)?;
            gauge.register(&mut gauge_registry.registry)?;
            gauge_registry.sensor_gauge_map.insert(*sensor, gauge);
        }

        Ok(gauge_registry)
    }

    pub fn set(&self, output: &bsec::Output) {
        if let Some(gauge) = self.sensor_gauge_map.get(&output.sensor) {
            gauge.set(output.signal, output.accuracy)
        }
    }

    pub fn gather(&self) -> Vec<MetricFamily> {
        self.registry.gather()
    }
}

#[cfg(test)]
pub mod tests {
    use prometheus::proto::{Gauge, Metric, MetricType};

    use super::*;

    #[test]
    fn test_bsec_gauge_registry() {
        let registry = BsecGaugeRegistry::new(&[bsec::OutputKind::Co2Equivalent]).unwrap();
        let tracked_output = bsec::Output {
            timestamp_ns: 0,
            signal: 42.,
            sensor: bsec::OutputKind::Co2Equivalent,
            accuracy: bsec::Accuracy::HighAccuracy,
        };
        let untracked_output = bsec::Output {
            timestamp_ns: 0,
            signal: 123.,
            sensor: bsec::OutputKind::RawGas,
            accuracy: bsec::Accuracy::HighAccuracy,
        };

        registry.set(&tracked_output);
        registry.set(&untracked_output);

        let mut metrics = registry.gather();
        metrics.sort_by(|a, b| a.get_name().cmp(b.get_name()));

        assert_eq!(
            metrics,
            [
                create_gauge_metric_family(
                    "co2_equivalent_accuracy".into(),
                    (bsec::Accuracy::HighAccuracy as u8).into(),
                    "CO2 equivalent estimate (accuracy)".into(),
                ),
                create_gauge_metric_family(
                    "co2_equivalent_ppm".into(),
                    42.,
                    "CO2 equivalent estimate (ppm)".into(),
                ),
            ]
        );
    }

    fn create_gauge_metric_family(name: String, value: f64, help: String) -> MetricFamily {
        let mut gauge = Gauge::new();
        gauge.set_value(value);

        let mut metric = Metric::new();
        metric.set_gauge(gauge);

        let mut family = MetricFamily::new();
        family.set_name(name);
        family.set_help(help);
        family.set_field_type(MetricType::GAUGE);
        family.set_metric(protobuf::RepeatedField::from_slice(&[metric]));
        family
    }
}
