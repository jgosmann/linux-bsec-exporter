use bme680_metrics_exporter::bsec::ffi::{
    bsec_bme_settings_t, bsec_do_steps, bsec_init, bsec_input_t, bsec_output_t,
    bsec_physical_sensor_t_BSEC_INPUT_GASRESISTOR, bsec_physical_sensor_t_BSEC_INPUT_HUMIDITY,
    bsec_physical_sensor_t_BSEC_INPUT_PRESSURE, bsec_physical_sensor_t_BSEC_INPUT_TEMPERATURE,
    bsec_sensor_configuration_t, bsec_sensor_control, bsec_update_subscription,
    bsec_virtual_sensor_t_BSEC_OUTPUT_CO2_EQUIVALENT, bsec_virtual_sensor_t_BSEC_OUTPUT_IAQ,
    bsec_virtual_sensor_t_BSEC_OUTPUT_RAW_PRESSURE,
    bsec_virtual_sensor_t_BSEC_OUTPUT_SENSOR_HEAT_COMPENSATED_HUMIDITY,
    bsec_virtual_sensor_t_BSEC_OUTPUT_SENSOR_HEAT_COMPENSATED_TEMPERATURE, BSEC_SAMPLE_RATE_LP,
};
use std::error::Error;
use std::fmt::Debug;
use std::fmt::Display;
use std::time::{Duration, SystemTime, UNIX_EPOCH};

use bme680::{Bme680, I2CAddress, OversamplingSetting, PowerMode, SettingsBuilder};
use linux_embedded_hal::{Delay, I2cdev};

#[derive(Debug)]
struct Bme680Error<R, W>(bme680::Error<R, W>);

impl<R: Debug, W: Debug> Display for Bme680Error<R, W> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> Result<(), std::fmt::Error> {
        f.write_fmt(format_args!("{:?}", self))
    }
}

impl<R: Debug, W: Debug> Error for Bme680Error<R, W> {}

fn main() -> Result<(), Box<dyn Error>> {
    unsafe {
        bsec_init();
        let conf = [
            bsec_sensor_configuration_t {
                sample_rate: BSEC_SAMPLE_RATE_LP as f32,
                sensor_id: bsec_virtual_sensor_t_BSEC_OUTPUT_CO2_EQUIVALENT as u8,
            },
            bsec_sensor_configuration_t {
                sample_rate: BSEC_SAMPLE_RATE_LP as f32,
                sensor_id: bsec_virtual_sensor_t_BSEC_OUTPUT_IAQ as u8,
            },
            bsec_sensor_configuration_t {
                sample_rate: BSEC_SAMPLE_RATE_LP as f32,
                sensor_id: bsec_virtual_sensor_t_BSEC_OUTPUT_RAW_PRESSURE as u8,
            },
            bsec_sensor_configuration_t {
                sample_rate: BSEC_SAMPLE_RATE_LP as f32,
                sensor_id: bsec_virtual_sensor_t_BSEC_OUTPUT_SENSOR_HEAT_COMPENSATED_HUMIDITY as u8,
            },
            bsec_sensor_configuration_t {
                sample_rate: BSEC_SAMPLE_RATE_LP as f32,
                sensor_id: bsec_virtual_sensor_t_BSEC_OUTPUT_SENSOR_HEAT_COMPENSATED_TEMPERATURE
                    as u8,
            },
        ];
        let mut required_sensor_settings: [bsec_sensor_configuration_t; 0] = [];
        let mut n_required_sensor_settings = 0;
        bsec_update_subscription(
            conf.as_ptr(),
            conf.len() as u8,
            required_sensor_settings.as_mut_ptr(),
            &mut n_required_sensor_settings,
        );
        let mut bme_settings = bsec_bme_settings_t {
            next_call: 0,
            process_data: 0,
            heater_temperature: 0,
            heating_duration: 0,
            run_gas: 0,
            pressure_oversampling: 0,
            temperature_oversampling: 0,
            humidity_oversampling: 0,
            trigger_measurement: 0,
        };
        loop {
            bsec_sensor_control(
                SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos() as i64,
                &mut bme_settings,
            );
            println!("{:?}", bme_settings);
            if bme_settings.trigger_measurement > 0 {
                let i2c = I2cdev::new("/dev/i2c-1")?;
                let mut dev =
                    Bme680::init(i2c, Delay {}, I2CAddress::Secondary).map_err(Bme680Error)?;
                let settings = SettingsBuilder::new()
                    .with_humidity_oversampling(OversamplingSetting::from_u8(
                        bme_settings.humidity_oversampling,
                    ))
                    .with_temperature_oversampling(OversamplingSetting::from_u8(
                        bme_settings.temperature_oversampling,
                    ))
                    .with_pressure_oversampling(OversamplingSetting::from_u8(
                        bme_settings.pressure_oversampling,
                    ))
                    .with_run_gas(bme_settings.run_gas > 0)
                    .with_gas_measurement(
                        Duration::from_millis(bme_settings.heating_duration.into()),
                        bme_settings.heater_temperature,
                        25,
                    )
                    .build();
                dev.set_sensor_settings(settings).map_err(Bme680Error)?;
                let profile_duration = dev.get_profile_dur(&settings.0).map_err(Bme680Error)?;
                dev.set_sensor_mode(PowerMode::ForcedMode)
                    .map_err(Bme680Error)?;
                std::thread::sleep(profile_duration);
                let (data, _state) = dev.get_sensor_data().map_err(Bme680Error)?;
                println!("Temp: {}°C", data.temperature_celsius());
                println!("Pressure: {}hPa", data.pressure_hpa());
                println!("Humidity: {}%", data.humidity_percent());
                println!("Gas resistance: {}ohm", data.gas_resistance_ohm());

                let time_stamp = SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos() as i64;
                let inputs = [
                    bsec_input_t {
                        time_stamp,
                        signal: data.temperature_celsius(),
                        signal_dimensions: 1,
                        sensor_id: bsec_physical_sensor_t_BSEC_INPUT_TEMPERATURE as u8,
                    },
                    bsec_input_t {
                        time_stamp,
                        signal: data.pressure_hpa(),
                        signal_dimensions: 1,
                        sensor_id: bsec_physical_sensor_t_BSEC_INPUT_PRESSURE as u8,
                    },
                    bsec_input_t {
                        time_stamp,
                        signal: data.humidity_percent(),
                        signal_dimensions: 1,
                        sensor_id: bsec_physical_sensor_t_BSEC_INPUT_HUMIDITY as u8,
                    },
                    bsec_input_t {
                        time_stamp,
                        signal: data.gas_resistance_ohm() as f32,
                        signal_dimensions: 1,
                        sensor_id: bsec_physical_sensor_t_BSEC_INPUT_GASRESISTOR as u8,
                    },
                ];
                let mut outputs = [
                    bsec_output_t {
                        time_stamp: 0,
                        signal: 0.,
                        signal_dimensions: 1,
                        sensor_id: bsec_virtual_sensor_t_BSEC_OUTPUT_CO2_EQUIVALENT as u8,
                        accuracy: 0,
                    },
                    bsec_output_t {
                        time_stamp: 0,
                        signal: 0.,
                        signal_dimensions: 1,
                        sensor_id: bsec_virtual_sensor_t_BSEC_OUTPUT_IAQ as u8,
                        accuracy: 0,
                    },
                    bsec_output_t {
                        time_stamp: 0,
                        signal: 0.,
                        signal_dimensions: 1,
                        sensor_id:
                            bsec_virtual_sensor_t_BSEC_OUTPUT_SENSOR_HEAT_COMPENSATED_HUMIDITY as u8,
                        accuracy: 0,
                    },
                    bsec_output_t {
                        time_stamp: 0,
                        signal: 0.,
                        signal_dimensions: 1,
                        sensor_id:
                            bsec_virtual_sensor_t_BSEC_OUTPUT_SENSOR_HEAT_COMPENSATED_TEMPERATURE
                                as u8,
                        accuracy: 0,
                    },
                    bsec_output_t {
                        time_stamp: 0,
                        signal: 0.,
                        signal_dimensions: 1,
                        sensor_id: bsec_virtual_sensor_t_BSEC_OUTPUT_RAW_PRESSURE as u8,
                        accuracy: 0,
                    },
                ];
                let mut n_outputs = outputs.len() as u8;
                bsec_do_steps(
                    inputs.as_ptr(),
                    inputs.len() as u8,
                    outputs.as_mut_ptr(),
                    &mut n_outputs,
                );
                for output in &outputs {
                    println!(
                        "{}: {} ({})",
                        match output.sensor_id as u32 {
                            bsec_virtual_sensor_t_BSEC_OUTPUT_SENSOR_HEAT_COMPENSATED_HUMIDITY => {
                                "Humidity (%)"
                            }
                            bsec_virtual_sensor_t_BSEC_OUTPUT_SENSOR_HEAT_COMPENSATED_TEMPERATURE =>
                            {
                                "Temp (°C)"
                            }
                            bsec_virtual_sensor_t_BSEC_OUTPUT_IAQ => {
                                "IAQ"
                            }
                            bsec_virtual_sensor_t_BSEC_OUTPUT_CO2_EQUIVALENT => {
                                "CO2"
                            }
                            bsec_virtual_sensor_t_BSEC_OUTPUT_RAW_PRESSURE => {
                                "Pressure (hPa)"
                            }
                            _ => "?",
                        },
                        output.signal,
                        output.accuracy,
                    );
                }
                if let Some(duration_ns) = (bme_settings.next_call as u128)
                    .checked_sub(SystemTime::now().duration_since(UNIX_EPOCH)?.as_nanos())
                {
                    std::thread::sleep(Duration::from_nanos(duration_ns as u64));
                }
            }
        }
    }
    println!("Hello, world!");
    Ok(())
}
