use bme680_metrics_exporter::bsec::Accuracy;
use bme680_metrics_exporter::bsec::BmeSensor;
use bme680_metrics_exporter::bsec::OutputSignal;
use bme680_metrics_exporter::bsec::Time;
use bme680_metrics_exporter::monitor::PersistState;
use bme680_metrics_exporter::monitor::Sleep;
use prometheus::proto::MetricFamily;
use prometheus::{Encoder, Gauge, Opts, Registry};
use std::collections::HashMap;
use std::convert::TryFrom;
use std::error::Error;
use std::fs::File;
use std::io::Read;
use std::sync::Arc;
use tokio::signal::unix::{signal, SignalKind};

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

    pub fn set(&self, value: f64, accuracy: Accuracy) {
        self.value.set(value);
        self.accuracy.set((accuracy as u8).into());
    }
}

impl TryFrom<&VirtualSensorOutput> for BsecGauge {
    type Error = prometheus::Error;

    fn try_from(sensor: &VirtualSensorOutput) -> Result<Self, Self::Error> {
        use VirtualSensorOutput::*;
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
    sensor_gauge_map: HashMap<VirtualSensorOutput, BsecGauge>,
}

impl BsecGaugeRegistry {
    pub fn new(sensors: &[VirtualSensorOutput]) -> prometheus::Result<Self> {
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
    pub fn set(&self, output: &OutputSignal) {
        if let Some(gauge) = self.sensor_gauge_map.get(&output.sensor) {
            gauge.set(output.signal, output.accuracy)
        }
    }
    pub fn gather(&self) -> Vec<MetricFamily> {
        self.registry.gather()
    }
}

const ACTIVE_SENSORS: [VirtualSensorOutput; 13] = [
    VirtualSensorOutput::Iaq,
    VirtualSensorOutput::StaticIaq,
    VirtualSensorOutput::Co2Equivalent,
    VirtualSensorOutput::BreathVocEquivalent,
    VirtualSensorOutput::RawTemperature,
    VirtualSensorOutput::RawPressure,
    VirtualSensorOutput::RawHumidity,
    VirtualSensorOutput::RawGas,
    VirtualSensorOutput::StabilizationStatus,
    VirtualSensorOutput::RunInStatus,
    VirtualSensorOutput::SensorHeatCompensatedTemperature,
    VirtualSensorOutput::SensorHeatCompensatedHumidity,
    VirtualSensorOutput::GasPercentage,
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

async fn run_monitoring(
    bsec: Bsec<Dev, TimeAlive, Arc<TimeAlive>>,
    registry: BsecGaugeRegistry,
) -> anyhow::Result<()> {
    let mut monitor = Monitor::start(
        bsec,
        StateFile::new("/var/lib/bsec-metrics-exporter/state.bin"),
        TIME.clone(),
    )
    .await?;

    tokio::task::spawn_local(handle_sigterm(monitor.request_shutdown));

    println!("BSEC monitoring started.");
    while let Ok(_) = monitor.current.changed().await {
        let outputs = monitor.current.borrow();
        for output in outputs.iter() {
            registry.set(output);
        }
    }

    println!("Waiting for BSEC monitoring shutdown ...");
    monitor.join_handle.await??;
    println!("BSEC monitoring shutdown complete.");
    Ok(())
}

#[tokio::main(flavor = "current_thread")]
pub async fn main() -> Result<(), Box<dyn Error>> {
    println!("Acquiring sensor ...");
    let mut bsec = Bsec::init(Dev::new()?, TIME.clone())?;
    println!("Setting config ...");
    let mut config = Vec::<u8>::new();
    File::open("/etc/bsec-metrics-exporter/bsec_iaq.config")?.read_to_end(&mut config)?;
    bsec.set_configuration(&config[4..])?; // First four bytes give config length
    println!("Sensor initialized.");
    let conf: Vec<_> = ACTIVE_SENSORS
        .iter()
        .map(|&sensor| RequestedSensorConfiguration {
            sample_rate: SampleRate::Lp,
            sensor,
        })
        .collect();
    bsec.update_subscription(&conf)?;
    let local = tokio::task::LocalSet::new();
    let registry = BsecGaugeRegistry::new(&ACTIVE_SENSORS)?;

    let monitoring = local.run_until(run_monitoring(bsec, registry.clone()));

    let mut app = tide::with_state(registry);
    app.at("/metrics").get(serve_metrics);
    println!("Spawning server ...");
    tokio::spawn(async move {
        app.listen("0.0.0.0:9118").await // FIXME spawn to allow parallelization?
    });

    monitoring.await?;
    println!("Shutdown.");

    Ok(())
}
