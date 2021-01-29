use bme680_metrics_exporter::bsec::Accuracy;
use prometheus::{Encoder, Gauge, Opts, Registry};
use std::error::Error;
use std::sync::Arc;

use bme680_metrics_exporter::bme680::Dev;
use bme680_metrics_exporter::bsec::{
    Bsec, RequestedSensorConfiguration, SampleRate, VirtualSensorOutput,
};
use bme680_metrics_exporter::monitor::Monitor;
use bme680_metrics_exporter::persistance::StateFile;
use bme680_metrics_exporter::time::TimeAlive;

#[macro_use]
extern crate lazy_static;

lazy_static! {
    static ref TIME: Arc<TimeAlive> = Arc::default();
}

struct BsecGauge {
    value: Gauge,
    accuracy: Gauge,
}

impl BsecGauge {
    fn new(name: &str, help: &str, unit: Option<&str>) -> prometheus::Result<Self> {
        let value = if let Some(unit) = unit {
            Gauge::with_opts(Opts::new(
                format!("{}_{}", name, unit),
                format!("{} ({})", help, unit),
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
    fn set(&self, value: f64, accuracy: Accuracy) {
        self.value.set(value);
        self.accuracy.set((accuracy as u8).into());
    }
}

#[tokio::main(flavor = "current_thread")]
pub async fn main() -> Result<(), Box<dyn Error>> {
    let mut bsec = Bsec::init(Dev::new()?, TIME.clone())?;
    let conf = vec![
        RequestedSensorConfiguration {
            sample_rate: SampleRate::Lp,
            sensor: VirtualSensorOutput::Co2Equivalent,
        },
        RequestedSensorConfiguration {
            sample_rate: SampleRate::Lp,
            sensor: VirtualSensorOutput::Iaq,
        },
        RequestedSensorConfiguration {
            sample_rate: SampleRate::Lp,
            sensor: VirtualSensorOutput::RawPressure,
        },
        RequestedSensorConfiguration {
            sample_rate: SampleRate::Lp,
            sensor: VirtualSensorOutput::SensorHeatCompensatedHumidity,
        },
        RequestedSensorConfiguration {
            sample_rate: SampleRate::Lp,
            sensor: VirtualSensorOutput::SensorHeatCompensatedTemperature,
        },
    ];
    bsec.update_subscription(&conf)?;
    let local = tokio::task::LocalSet::new();
    let registry = Registry::new();
    let co2_equivalent = BsecGauge::new("co2_equivalent", "CO2 equivalent", Some("ppm"))?;
    let iaq = BsecGauge::new("iaq", "Indoor air quality index", None)?;
    let pressure = BsecGauge::new("pressure", "Pressure", Some("hPa"))?;
    let humidity = BsecGauge::new(
        "humidity",
        "Sensor heat compensated humidity",
        Some("percent"),
    )?;
    let temperature = BsecGauge::new(
        "temperature",
        "Sesor heat compensated temperature",
        Some("celsius"),
    )?;
    co2_equivalent.register(&registry)?;
    iaq.register(&registry)?;
    pressure.register(&registry)?;
    humidity.register(&registry)?;
    temperature.register(&registry)?;

    local
        .run_until(async move {
            let mut monitor = Monitor::start(
                bsec,
                StateFile::new("/var/lib/bsec-metrics-exporter/state.bin"),
                TIME.clone(),
            )
            .await
            .unwrap();
            loop {
                monitor.current.changed().await.unwrap();
                let outputs = monitor.current.borrow();
                for output in outputs.iter() {
                    match output.sensor {
                        VirtualSensorOutput::SensorHeatCompensatedHumidity => {
                            humidity.set(output.signal, output.accuracy);
                        }
                        VirtualSensorOutput::SensorHeatCompensatedTemperature => {
                            temperature.set(output.signal, output.accuracy);
                        }
                        VirtualSensorOutput::Iaq => {
                            iaq.set(output.signal, output.accuracy);
                        }
                        VirtualSensorOutput::Co2Equivalent => {
                            co2_equivalent.set(output.signal, output.accuracy);
                        }
                        VirtualSensorOutput::RawPressure => {
                            pressure.set(output.signal, output.accuracy);
                        }
                        _ => (),
                    };
                }
                let mut buffer = vec![];
                let encoder = prometheus::TextEncoder::new();
                encoder.encode(&registry.gather(), &mut buffer).unwrap(); // FIXME
                println!("{}", String::from_utf8(buffer).unwrap());
            }
        })
        .await;

    Ok(())
}
