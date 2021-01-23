use std::error::Error;
use std::fmt::Debug;
use std::fmt::Display;
use std::time::{Duration, SystemTime};

use bme680::{Bme680, I2CAddress, OversamplingSetting, PowerMode, SettingsBuilder};
use bme680_metrics_exporter::bsec::{
    BmeOutput, BmeSensor, BmeSettingsHandle, Bsec, PhysicalSensorInput,
    RequestedSensorConfiguration, SampleRate, Time, VirtualSensorOutput,
};
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
    start: SystemTime,
}

impl TimeAlive {
    fn new() -> Self {
        TimeAlive {
            start: SystemTime::now(),
        }
    }
    fn wait(&self, timestamp_ns: i64) {
        let duration = Duration::from_nanos((timestamp_ns - self.timestamp_ns()) as u64);
        std::thread::sleep(duration);
    }
}

impl Time for TimeAlive {
    fn timestamp_ns(&self) -> i64 {
        SystemTime::now()
            .duration_since(self.start)
            .unwrap()
            .as_nanos() as i64
    }
}

struct Dev {
    dev: Bme680<linux_embedded_hal::I2cdev, linux_embedded_hal::Delay>,
}

impl Dev {
    fn new() -> Result<Self, Box<dyn Error>> {
        let i2c = I2cdev::new("/dev/i2c-1")?;
        Ok(Dev {
            dev: Bme680::init(i2c, Delay {}, I2CAddress::Secondary).map_err(Bme680Error)?,
        })
    }
}

impl BmeSensor for Dev {
    type Error = Bme680Error<
        linux_embedded_hal::i2cdev::linux::LinuxI2CError,
        linux_embedded_hal::i2cdev::linux::LinuxI2CError,
    >;
    fn perform_measurement(
        &mut self,
        settings: &BmeSettingsHandle,
    ) -> Result<Vec<BmeOutput>, Self::Error> {
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
        std::thread::sleep(profile_duration);
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

fn main() -> Result<(), Box<dyn Error>> {
    let time = TimeAlive::new();
    let mut bsec = Bsec::init(Dev::new()?, &time)?;
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
    loop {
        let outputs = bsec.do_step()?;
        for output in outputs.signals.iter() {
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
        time.wait(outputs.next_call);
    }
}
