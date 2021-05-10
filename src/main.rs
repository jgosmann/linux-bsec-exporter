use linux_embedded_hal::{Delay, I2cdev};
use prometheus::proto::MetricFamily;
use prometheus::{Encoder, Gauge, Opts, Registry};
use std::collections::HashMap;
use std::convert::TryFrom;
use std::error::Error;
use std::fs::File;
use std::io::Read;
use std::sync::Arc;
use tokio::signal::unix::{signal, SignalKind};

use bsec::bme::bme680::Bme680Sensor;
use bsec::clock::TimePassed;
use linux_bsec_exporter::monitor::bsec_monitor;
use linux_bsec_exporter::persistance::StateFile;

#[macro_use]
extern crate lazy_static;

lazy_static! {
    static ref TIME: Arc<TimePassed> = Arc::default();
}

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
    pub fn new(name: &str, help: &str, unit: Option<&GaugeUnit>) -> prometheus::Result<Self> {
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

    pub fn register(&self, registry: &Registry) -> prometheus::Result<()> {
        registry.register(Box::new(self.value.clone()))?;
        registry.register(Box::new(self.accuracy.clone()))?;
        Ok(())
    }

    pub fn set(&self, value: f64, accuracy: bsec::Accuracy) {
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
struct BsecGaugeRegistry {
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

const ACTIVE_SENSORS: [bsec::OutputKind; 13] = [
    bsec::OutputKind::Iaq,
    bsec::OutputKind::StaticIaq,
    bsec::OutputKind::Co2Equivalent,
    bsec::OutputKind::BreathVocEquivalent,
    bsec::OutputKind::RawTemperature,
    bsec::OutputKind::RawPressure,
    bsec::OutputKind::RawHumidity,
    bsec::OutputKind::RawGas,
    bsec::OutputKind::StabilizationStatus,
    bsec::OutputKind::RunInStatus,
    bsec::OutputKind::SensorHeatCompensatedTemperature,
    bsec::OutputKind::SensorHeatCompensatedHumidity,
    bsec::OutputKind::GasPercentage,
];

async fn serve_metrics(req: tide::Request<BsecGaugeRegistry>) -> tide::Result {
    let mut buffer = vec![];
    let encoder = prometheus::TextEncoder::new();
    encoder.encode(&req.state().gather(), &mut buffer)?;
    Ok(format!("{}", String::from_utf8(buffer)?).into())
}

async fn handle_sigterm(request_shutdown: tokio::sync::oneshot::Sender<()>) -> std::io::Result<()> {
    signal(SignalKind::terminate())?.recv().await;
    let _ = request_shutdown.send(());
    Ok(())
}

type Dev = Bme680Sensor<linux_embedded_hal::I2cdev, linux_embedded_hal::Delay>;

async fn run_monitoring(
    bsec: bsec::Bsec<Dev, TimePassed, Arc<TimePassed>>,
    registry: BsecGaugeRegistry,
) -> anyhow::Result<()> {
    let (monitor, mut rx) = bsec_monitor(
        bsec,
        StateFile::new("/var/lib/bsec-metrics-exporter/state.bin"),
        TIME.clone(),
    );
    let join_handle = tokio::task::spawn(monitor.monitoring_loop());

    tokio::task::spawn(handle_sigterm(rx.initiate_shutdown));

    println!("BSEC monitoring started.");
    while let Ok(_) = rx.current.changed().await {
        if let Some(outputs) = rx.current.borrow().as_deref() {
            for output in outputs.iter() {
                registry.set(output);
            }
        }
    }

    println!("Waiting for BSEC monitoring shutdown ...");
    join_handle.await??;
    println!("BSEC monitoring shutdown complete.");
    Ok(())
}

#[tokio::main(flavor = "current_thread")]
pub async fn main() -> Result<(), Box<dyn Error>> {
    println!("Acquiring sensor ...");
    let i2c = I2cdev::new("/dev/i2c-1")?;
    let dev = bme680::Bme680::init(i2c, Delay {}, bme680::I2CAddress::Secondary).unwrap();
    let sensor = bsec::bme::bme680::Bme680Sensor::new(dev, 20.);
    let mut bsec = bsec::Bsec::init(sensor, TIME.clone())?;
    println!("Setting config ...");
    let mut config = Vec::<u8>::new();
    File::open("/etc/bsec-metrics-exporter/bsec_iaq.config")?.read_to_end(&mut config)?;
    bsec.set_configuration(&config[4..])?; // First four bytes give config length
    println!("Sensor initialized.");
    let conf: Vec<_> = ACTIVE_SENSORS
        .iter()
        .map(|&sensor| bsec::SubscriptionRequest {
            sample_rate: bsec::SampleRate::Lp,
            sensor,
        })
        .collect();
    bsec.update_subscription(&conf)?;
    let registry = BsecGaugeRegistry::new(&ACTIVE_SENSORS)?;

    let join_handle = tokio::task::spawn(run_monitoring(bsec, registry.clone()));

    let mut app = tide::with_state(registry);
    app.at("/metrics").get(serve_metrics);
    println!("Spawning server ...");
    app.listen("0.0.0.0:9118").await?;
    join_handle.await??;
    println!("Shutdown.");

    Ok(())
}
