use linux_embedded_hal::{Delay, I2cdev};
use prometheus::Encoder;
use std::error::Error;
use std::fs::{self, File};
use std::io::Read;
use std::sync::Arc;
use tokio::signal::unix::{signal, Signal, SignalKind};

use bsec::clock::TimePassed;
use bsec::{bme::bme680::Bme680Sensor, OutputKind};
use linux_bsec_exporter::persistance::StateFile;
use linux_bsec_exporter::{metrics::BsecGaugeRegistry, monitor::bsec_monitor};

#[macro_use]
extern crate lazy_static;

lazy_static! {
    static ref TIME: Arc<TimePassed> = Arc::default();
}

async fn serve_metrics(req: tide::Request<BsecGaugeRegistry>) -> tide::Result {
    let mut buffer = vec![];
    let encoder = prometheus::TextEncoder::new();
    encoder.encode(&req.state().gather(), &mut buffer)?;
    Ok(format!("{}", String::from_utf8(buffer)?).into())
}

struct SigTermHandler(Signal);

impl SigTermHandler {
    pub fn new() -> std::io::Result<Self> {
        Ok(Self(signal(SignalKind::terminate())?))
    }

    pub async fn dispatch_to(mut self, sender: tokio::sync::oneshot::Sender<()>) {
        self.0.recv().await;
        let _ = sender.send(());
    }
}

type Dev = Bme680Sensor<linux_embedded_hal::I2cdev, linux_embedded_hal::Delay>;

async fn run_monitoring(
    bsec: bsec::Bsec<Dev, TimePassed, Arc<TimePassed>>,
    registry: BsecGaugeRegistry,
) -> anyhow::Result<()> {
    let (monitor, mut rx) = bsec_monitor(
        bsec,
        StateFile::new("/var/lib/bsec-metrics-exporter/state.bin"), // FIXME take from config
        TIME.clone(),
    );

    tokio::task::spawn(SigTermHandler::new()?.dispatch_to(rx.initiate_shutdown));
    let join_handle = tokio::task::spawn(monitor.monitoring_loop());

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
    let config: linux_bsec_exporter::config::Config =
        toml::from_str(&fs::read_to_string("/etc/linux-bsec-exporter/config.toml")?)?;

    println!("Initializing sensor ...");
    let i2c = I2cdev::new(config.sensor.device)?;
    let dev = bme680::Bme680::init(i2c, Delay {}, config.sensor.address).unwrap(); // FIXME error handling
    let sensor = bsec::bme::bme680::Bme680SensorBuilder::new(dev)
        .initial_ambient_temp_celsius(config.sensor.initial_ambient_temp_celsius)
        .temp_offset_celsius(config.bsec.temperature_offset_celsius)
        .build();
    let mut bsec = bsec::Bsec::init(sensor, TIME.clone())?;

    println!("Setting BSEC config ...");
    let mut bsec_config = Vec::<u8>::new();
    File::open(config.bsec.config)?.read_to_end(&mut bsec_config)?;
    bsec.set_configuration(&bsec_config[4..])?; // First four bytes give config length

    println!("Subscribing to BSEC outputs ...");
    bsec.update_subscription(&config.bsec.subscriptions)?;
    let registry = BsecGaugeRegistry::new(
        &config
            .bsec
            .subscriptions
            .iter()
            .map(|item| item.sensor)
            .collect::<Vec<OutputKind>>(),
    )?;

    let join_handle = tokio::task::spawn(run_monitoring(bsec, registry.clone()));

    let mut app = tide::with_state(registry);
    app.at("/metrics").get(serve_metrics);
    println!("Spawning server ...");
    app.listen("0.0.0.0:9118").await?;
    join_handle.await??;
    println!("Shutdown.");

    Ok(())
}
