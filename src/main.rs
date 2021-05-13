use linux_embedded_hal::{Delay, I2cdev};
use prometheus::Encoder;
use std::error::Error;
use std::fs::File;
use std::io::Read;
use std::sync::Arc;
use tokio::signal::unix::{signal, SignalKind};

use bsec::bme::bme680::Bme680Sensor;
use bsec::clock::TimePassed;
use linux_bsec_exporter::persistance::StateFile;
use linux_bsec_exporter::{metrics::BsecGaugeRegistry, monitor::bsec_monitor};

#[macro_use]
extern crate lazy_static;

lazy_static! {
    static ref TIME: Arc<TimePassed> = Arc::default();
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
