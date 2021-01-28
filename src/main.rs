use bme680_metrics_exporter::monitor::PersistState;
use std::error::Error;
use std::fmt::Debug;
use std::fmt::Display;
use std::sync::Arc;
use std::time::{Duration, Instant};

use bme680::{Bme680, I2CAddress, OversamplingSetting, PowerMode, SettingsBuilder};
use bme680_metrics_exporter::bsec::{
    BmeOutput, BmeSensor, BmeSettingsHandle, Bsec, PhysicalSensorInput,
    RequestedSensorConfiguration, SampleRate, Time, VirtualSensorOutput,
};
use bme680_metrics_exporter::monitor::Monitor;
use linux_embedded_hal::{Delay, I2cdev};

#[derive(Debug)]
struct Bme680Error<R, W>(bme680::Error<R, W>);

impl<R: Debug, W: Debug> Display for Bme680Error<R, W> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> Result<(), std::fmt::Error> {
        f.write_fmt(format_args!("{:?}", self))
    }
}

impl<R: Debug, W: Debug> Error for Bme680Error<R, W> {}

struct TimeAlive {
    start: Instant,
}

impl Default for TimeAlive {
    fn default() -> Self {
        TimeAlive {
            start: Instant::now(),
        }
    }
}

impl Time for TimeAlive {
    fn timestamp_ns(&self) -> i64 {
        Instant::now().duration_since(self.start).as_nanos() as i64
    }
}

struct Dev {
    dev: Bme680<linux_embedded_hal::I2cdev, linux_embedded_hal::Delay>,
    measurement_available_after: Option<Instant>,
}

impl Dev {
    fn new() -> Result<Self, Box<dyn Error>> {
        let i2c = I2cdev::new("/dev/i2c-1")?;
        Ok(Dev {
            dev: Bme680::init(i2c, Delay {}, I2CAddress::Secondary).map_err(Bme680Error)?,
            measurement_available_after: None,
        })
    }
}

impl BmeSensor for Dev {
    type Error = Bme680Error<
        linux_embedded_hal::i2cdev::linux::LinuxI2CError,
        linux_embedded_hal::i2cdev::linux::LinuxI2CError,
    >;
    fn start_measurement(
        &mut self,
        settings: &BmeSettingsHandle,
    ) -> Result<std::time::Duration, Self::Error> {
        let settings = SettingsBuilder::new()
            .with_humidity_oversampling(OversamplingSetting::from_u8(
                settings.humidity_oversampling(),
            ))
            .with_temperature_oversampling(OversamplingSetting::from_u8(
                settings.temperature_oversampling(),
            ))
            .with_pressure_oversampling(OversamplingSetting::from_u8(
                settings.pressure_oversampling(),
            ))
            .with_run_gas(settings.run_gas())
            .with_gas_measurement(
                Duration::from_millis(settings.heating_duration().into()),
                settings.heater_temperature(),
                25,
            )
            .build();
        self.dev
            .set_sensor_settings(settings)
            .map_err(Bme680Error)?;
        let profile_duration = self.dev.get_profile_dur(&settings.0).map_err(Bme680Error)?;
        self.dev
            .set_sensor_mode(PowerMode::ForcedMode)
            .map_err(Bme680Error)?;
        self.measurement_available_after = Some(std::time::Instant::now() + profile_duration);
        Ok(profile_duration)
    }

    fn get_measurement(&mut self) -> nb::Result<Vec<BmeOutput>, Self::Error> {
        match self.measurement_available_after {
            None => panic!("Mast call start_measurement before get_measurement."),
            Some(instant) if instant > std::time::Instant::now() => Err(nb::Error::WouldBlock),
            _ => {
                let (data, _state) = self.dev.get_sensor_data().map_err(Bme680Error)?;
                Ok(vec![
                    BmeOutput {
                        sensor: PhysicalSensorInput::Temperature,
                        signal: data.temperature_celsius(),
                    },
                    BmeOutput {
                        sensor: PhysicalSensorInput::Pressure,
                        signal: data.pressure_hpa(),
                    },
                    BmeOutput {
                        sensor: PhysicalSensorInput::Humidity,
                        signal: data.humidity_percent(),
                    },
                    BmeOutput {
                        sensor: PhysicalSensorInput::GasResistor,
                        signal: data.gas_resistance_ohm() as f32,
                    },
                ])
            }
        }
    }
}

#[derive(Default)]
pub struct NoPersistState {}
impl PersistState for NoPersistState {
    type Error = std::convert::Infallible;
    fn load_state(&mut self) -> Result<Option<Vec<u8>>, Self::Error> {
        Ok(None)
    }
    fn save_state(&mut self, _: &[u8]) -> Result<(), Self::Error> {
        Ok(())
    }
}

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
