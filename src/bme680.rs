use std::error::Error;
use std::fmt::Debug;
use std::fmt::Display;
use std::time::{Duration, Instant};

use super::bsec::{BmeOutput, BmeSensor, BmeSettingsHandle, PhysicalSensorInput};
use bme680::{Bme680, I2CAddress, OversamplingSetting, PowerMode, SettingsBuilder};
use linux_embedded_hal::{Delay, I2cdev};

#[derive(Debug)]
pub struct Bme680Error<R, W>(bme680::Error<R, W>);

impl<R: Debug, W: Debug> Display for Bme680Error<R, W> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> Result<(), std::fmt::Error> {
        use bme680::Error::*;
        match &self.0 {
            I2CWrite(w) => f.write_fmt(format_args!("I2C write error: {:?}", w)),
            I2CRead(r) => f.write_fmt(format_args!("I2C read error: {:?}", r)),
            DeviceNotFound => f.write_str("DeviceNotFound/BME680_E_DEV_NOT_FOUND"),
            InvalidLength => f.write_str("InvalidLength/BME680_E_INVALID_LENGTH"),
            DefinePwrMode => f.write_str("DefinePwrMode/BME680_W_DEFINE_PWR_MODE"),
            NoNewData => f.write_str("NoNewData/BME680_W_DEFINE_PWR_MODE"),
            BoundaryCheckFailure(msg) => f.write_fmt(format_args!("BoundaryCheckFailure: {}", msg)),
        }
    }
}

impl<R: Debug, W: Debug> Error for Bme680Error<R, W> {}

pub struct Dev {
    dev: Bme680<linux_embedded_hal::I2cdev, linux_embedded_hal::Delay>,
    measurement_available_after: Option<Instant>,
}

impl Dev {
    pub fn new() -> Result<Self, Box<dyn Error>> {
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
