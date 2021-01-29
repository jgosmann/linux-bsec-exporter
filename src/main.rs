use std::error::Error;
use std::sync::Arc;

use bme680_metrics_exporter::bme680::Dev;
use bme680_metrics_exporter::bsec::{
    Bsec, RequestedSensorConfiguration, SampleRate, VirtualSensorOutput,
};
use bme680_metrics_exporter::monitor::Monitor;
use bme680_metrics_exporter::persistance::NoPersistState;
use bme680_metrics_exporter::time::TimeAlive;

#[macro_use]
extern crate lazy_static;

lazy_static! {
    static ref TIME: Arc<TimeAlive> = Arc::default();
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

    local
        .run_until(async move {
            let mut monitor = Monitor::start(bsec, NoPersistState::default(), TIME.clone())
                .await
                .unwrap();
            loop {
                monitor.current.changed().await.unwrap();
                let outputs = monitor.current.borrow();
                for output in outputs.iter() {
                    println!(
                        "{}: {} ({:?})",
                        match output.sensor {
                            VirtualSensorOutput::SensorHeatCompensatedHumidity => {
                                "Humidity (%)"
                            }
                            VirtualSensorOutput::SensorHeatCompensatedTemperature => {
                                "Temp (°C)"
                            }
                            VirtualSensorOutput::Iaq => "IAQ",
                            VirtualSensorOutput::Co2Equivalent => "CO2",
                            VirtualSensorOutput::RawPressure => "Pressure (hPa)",
                            _ => "?",
                        },
                        output.signal,
                        output.accuracy,
                    );
                }
            }
        })
        .await;

    Ok(())
}
