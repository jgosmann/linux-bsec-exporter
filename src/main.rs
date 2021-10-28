use embedded_hal::blocking::i2c;
use libsystemd::daemon::{self, NotifyState};
use linux_embedded_hal::{Delay, I2cdev};
use prometheus::Encoder;
use std::error::Error;
use std::fs::{self, File};
use std::io::Read;
use std::sync::Arc;
use tokio::signal::unix::{signal, Signal, SignalKind};

use bsec::clock::TimePassed;
use bsec::{bme::bme680::Bme680Sensor, OutputKind};
use linux_bsec_exporter::middleware::LogErrors;
use linux_bsec_exporter::{metrics::BsecGaugeRegistry, monitor::bsec_monitor};
use linux_bsec_exporter::{monitor::PersistState, persistance::StateFile};

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

type SensorDevice = Bme680Sensor<linux_embedded_hal::I2cdev, linux_embedded_hal::Delay>;

async fn run_monitoring<P>(
    bsec: bsec::Bsec<SensorDevice, TimePassed, Arc<TimePassed>>,
    persistence: P,
    registry: BsecGaugeRegistry,
) -> anyhow::Result<()>
where
    P: PersistState + Send + Sync + 'static,
    P::Error: std::error::Error + Send + Sync + 'static,
{
    let (monitor, mut rx) = bsec_monitor(bsec, persistence, TIME.clone());

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

#[derive(Debug)]
struct Bme680Error(bme680::Error<<I2cdev as i2c::Read>::Error, <I2cdev as i2c::Write>::Error>);

impl std::fmt::Display for Bme680Error {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        f.write_fmt(format_args!("{:?}", self))
    }
}

impl std::error::Error for Bme680Error {}

#[tokio::main(flavor = "current_thread")]
pub async fn main() -> Result<(), Box<dyn Error>> {
    let config: linux_bsec_exporter::config::Config = toml::from_str(&fs::read_to_string(
        std::env::var("BSEC_CONFIG_PATH").unwrap_or("/etc/linux-bsec-exporter/config.toml".into()),
    )?)?;

    println!("Initializing sensor ...");
    let i2c = I2cdev::new(config.sensor.device)?;
    let dev = bme680::Bme680::init(i2c, Delay {}, config.sensor.address).map_err(Bme680Error)?;
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
    let monitoring = run_monitoring(
        bsec,
        StateFile::new(config.bsec.state_file),
        registry.clone(),
    );

    let mut app = tide::with_state(registry);
    app.with(LogErrors);
    app.at("/metrics").get(serve_metrics);
    println!("Spawning server ...");
    let join_handle = tokio::task::spawn(app.listen(config.exporter.listen_addrs));

    println!("Ready.");
    if daemon::booted() {
        daemon::notify(false, &[NotifyState::Ready])?;
    }

    tokio::select! {
        result = join_handle => result??,
        result = monitoring => result?,
    }

    if daemon::booted() {
        daemon::notify(true, &[NotifyState::Stopping])?;
    }
    println!("Shutdown.");

    Ok(())
}
