use self::ffi::*;
use core::convert::{From, TryFrom, TryInto};
use core::sync::atomic::{AtomicBool, Ordering};
use core::time::Duration;

static BSEC_IN_USE: AtomicBool = AtomicBool::new(false);

pub trait Time {
    fn timestamp_ns(&self) -> i64;
}

pub struct BmeSettingsHandle<'a> {
    bme_settings: &'a bsec_bme_settings_t,
}

impl<'a> BmeSettingsHandle<'a> {
    fn new(bme_settings: &'a bsec_bme_settings_t) -> Self {
        Self { bme_settings }
    }
    pub fn heater_temperature(&self) -> u16 {
        self.bme_settings.heater_temperature
    }
    pub fn heating_duration(&self) -> u16 {
        self.bme_settings.heating_duration
    }
    pub fn run_gas(&self) -> bool {
        self.bme_settings.run_gas == 1
    }
    pub fn pressure_oversampling(&self) -> u8 {
        self.bme_settings.pressure_oversampling
    }
    pub fn temperature_oversampling(&self) -> u8 {
        self.bme_settings.temperature_oversampling
    }
    pub fn humidity_oversampling(&self) -> u8 {
        self.bme_settings.humidity_oversampling
    }
}

#[derive(Clone, Debug)]
pub struct BmeOutput {
    pub signal: f32,
    pub sensor: PhysicalSensorInput,
}

pub trait BmeSensor {
    type Error;
    fn start_measurement(&mut self, settings: &BmeSettingsHandle) -> Result<Duration, Self::Error>;
    fn get_measurement(&mut self) -> nb::Result<Vec<BmeOutput>, Self::Error>;
}

pub struct Bsec<'t, S: BmeSensor, T: Time> {
    bme: S,
    subscribed: u32,
    ulp_plus_queue: u32,
    next_measurement: i64,
    time: &'t T,
}

impl<'t, S: BmeSensor, T: Time> Bsec<'t, S, T> {
    pub fn init(bme: S, time: &'t T) -> Result<Self, Error<S::Error>> {
        if !BSEC_IN_USE.compare_and_swap(false, true, Ordering::SeqCst) {
            unsafe {
                bsec_init().into_result()?;
            }
            Ok(Self {
                bme,
                subscribed: 0,
                ulp_plus_queue: 0,
                next_measurement: time.timestamp_ns(),
                time,
            })
        } else {
            Err(Error::BsecAlreadyInUse)
        }
    }

    pub fn update_subscription(
        &mut self,
        requested_outputs: &[RequestedSensorConfiguration],
    ) -> Result<Vec<RequiredSensorSettings>, Error<S::Error>> {
        let bsec_requested_outputs: Vec<bsec_sensor_configuration_t> =
            requested_outputs.iter().map(From::from).collect();
        let mut required_sensor_settings = [bsec_sensor_configuration_t {
            sample_rate: 0.,
            sensor_id: 0,
        }; ffi::BSEC_MAX_PHYSICAL_SENSOR as usize];
        let mut n_required_sensor_settings = ffi::BSEC_MAX_PHYSICAL_SENSOR as u8;
        unsafe {
            bsec_update_subscription(
                bsec_requested_outputs.as_ptr(),
                requested_outputs
                    .len()
                    .try_into()
                    .or(Err(Error::ArgumentListTooLong))?,
                required_sensor_settings.as_mut_ptr(),
                &mut n_required_sensor_settings,
            )
            .into_result()?
        }
        for changed in requested_outputs.iter() {
            match changed.sample_rate {
                SampleRate::Disabled => {
                    self.subscribed &= !(changed.sensor as u32);
                    self.ulp_plus_queue &= !(changed.sensor as u32);
                }
                SampleRate::UlpMeasurementOnDemand => {
                    self.ulp_plus_queue |= changed.sensor as u32;
                }
                _ => {
                    self.subscribed |= changed.sensor as u32;
                }
            }
        }
        // FIXME why are we getting invalid sensor ids?
        Ok(required_sensor_settings
            .iter()
            .take(n_required_sensor_settings as usize)
            .filter_map(|x| RequiredSensorSettings::try_from(x).ok())
            .collect())
    }
    pub fn next_measurement(&self) -> i64 {
        self.next_measurement
    }

    pub fn start_next_measurement(&mut self) -> nb::Result<Duration, Error<S::Error>> {
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
        unsafe {
            bsec_sensor_control(self.time.timestamp_ns(), &mut bme_settings)
                .into_result()
                .map_err(Error::BsecError)?;
        }
        self.next_measurement = bme_settings.next_call;
        if bme_settings.trigger_measurement != 1 {
            return Err(nb::Error::WouldBlock);
        }
        self.bme
            .start_measurement(&BmeSettingsHandle::new(&bme_settings))
            .map_err(Error::BmeSensorError)
            .map_err(nb::Error::Other)
    }
    pub fn process_last_measurement(&mut self) -> nb::Result<Vec<OutputSignal>, Error<S::Error>> {
        let time_stamp = self.time.timestamp_ns(); // FIXME provide timestamp closer to measurement?
        let inputs: Vec<bsec_input_t> = self
            .bme
            .get_measurement()
            .map_err(|e| e.map(Error::BmeSensorError))?
            .iter()
            .map(|o| bsec_input_t {
                time_stamp,
                signal: o.signal,
                signal_dimensions: 1,
                sensor_id: o.sensor.into(),
            })
            .collect();
        let mut outputs = vec![
            bsec_output_t {
                time_stamp: 0,
                signal: 0.,
                signal_dimensions: 1,
                sensor_id: 0,
                accuracy: 0,
            };
            (self.subscribed | self.ulp_plus_queue).count_ones() as usize
        ];
        let mut num_outputs: u8 = outputs
            .len()
            .try_into()
            .or(Err(Error::ArgumentListTooLong))?;
        self.ulp_plus_queue = 0;
        unsafe {
            bsec_do_steps(
                inputs.as_ptr(),
                inputs
                    .len()
                    .try_into()
                    .or(Err(Error::ArgumentListTooLong))?,
                outputs.as_mut_ptr(),
                &mut num_outputs,
            )
            .into_result()
            .map_err(Error::BsecError)?;
        }

        let signals: Result<Vec<OutputSignal>, Error<S::Error>> = outputs
            .iter()
            .take(num_outputs.into())
            .map(|x| OutputSignal::try_from(x).map_err(Error::<S::Error>::from))
            .collect();
        Ok(signals?)
    }

    pub fn get_state(&self) -> Result<Vec<u8>, Error<S::Error>> {
        let mut state = [0u8; BSEC_MAX_STATE_BLOB_SIZE as usize];
        let mut work_buffer = [0u8; BSEC_MAX_WORKBUFFER_SIZE as usize];
        let mut state_length = BSEC_MAX_STATE_BLOB_SIZE;
        unsafe {
            bsec_get_state(
                0,
                state.as_mut_ptr(),
                state.len() as u32,
                work_buffer.as_mut_ptr(),
                work_buffer.len() as u32,
                &mut state_length,
            )
            .into_result()?;
        }
        Ok(state[..state_length as usize].into())
    }

    pub fn set_state(&mut self, state: &[u8]) -> Result<(), Error<S::Error>> {
        let mut work_buffer = [0u8; BSEC_MAX_WORKBUFFER_SIZE as usize];
        unsafe {
            bsec_set_state(
                state.as_ptr(),
                state.len() as u32,
                work_buffer.as_mut_ptr(),
                work_buffer.len() as u32,
            )
            .into_result()?;
        }
        Ok(())
    }
    pub fn get_configuration(&self) -> Result<Vec<u8>, Error<S::Error>> {
        let mut serialized_settings = [0u8; BSEC_MAX_PROPERTY_BLOB_SIZE as usize];
        let mut serialized_settings_length = 0u32;
        let mut work_buffer = [0u8; BSEC_MAX_WORKBUFFER_SIZE as usize];
        unsafe {
            bsec_get_configuration(
                0,
                serialized_settings.as_mut_ptr(),
                serialized_settings.len() as u32,
                work_buffer.as_mut_ptr(),
                work_buffer.len() as u32,
                &mut serialized_settings_length,
            )
            .into_result()?;
        }
        Ok(serialized_settings[..serialized_settings_length as usize].into())
    }

    pub fn set_configuration(&mut self, serialized_settings: &[u8]) -> Result<(), Error<S::Error>> {
        let mut work_buffer = [0u8; BSEC_MAX_WORKBUFFER_SIZE as usize];
        unsafe {
            bsec_set_configuration(
                serialized_settings.as_ptr(),
                serialized_settings.len() as u32,
                work_buffer.as_mut_ptr(),
                work_buffer.len() as u32,
            )
            .into_result()?
        }
        Ok(())
    }

    pub fn reset_output(&mut self, sensor: VirtualSensorOutput) -> Result<(), Error<S::Error>> {
        unsafe {
            bsec_reset_output(bsec_virtual_sensor_t::from(sensor) as u8).into_result()?;
        }
        Ok(())
    }
}

pub fn get_version() -> Result<(u8, u8, u8, u8), BsecError> {
    let mut version = bsec_version_t {
        major: 0,
        minor: 0,
        major_bugfix: 0,
        minor_bugfix: 0,
    };
    unsafe {
        bsec_get_version(&mut version).into_result()?;
    }
    Ok((
        version.major,
        version.minor,
        version.major_bugfix,
        version.minor_bugfix,
    ))
}

impl<'t, S: BmeSensor, T: Time> Drop for Bsec<'t, S, T> {
    fn drop(&mut self) {
        BSEC_IN_USE.store(false, Ordering::SeqCst);
    }
}

#[derive(Clone, Debug)]
pub struct Output {
    pub next_call: i64,
    pub signals: Vec<OutputSignal>,
}

#[derive(Clone, Copy, Debug)]
pub struct OutputSignal {
    pub timestamp_ns: i64,
    pub signal: f64,
    pub sensor: VirtualSensorOutput,
    pub accuracy: Accuracy,
}

impl TryFrom<&bsec_output_t> for OutputSignal {
    type Error = ConversionError;
    fn try_from(output: &bsec_output_t) -> Result<Self, ConversionError> {
        Ok(Self {
            timestamp_ns: output.time_stamp,
            signal: output.signal.into(),
            sensor: output.sensor_id.try_into()?,
            accuracy: output.accuracy.try_into()?,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum Accuracy {
    Unreliable,
    LowAccuracy,
    MediumAccuracy,
    HighAccuracy,
}

impl TryFrom<u8> for Accuracy {
    type Error = ConversionError;
    fn try_from(accuracy: u8) -> Result<Self, ConversionError> {
        use Accuracy::*;
        match accuracy {
            0 => Ok(Unreliable),
            1 => Ok(LowAccuracy),
            2 => Ok(MediumAccuracy),
            3 => Ok(HighAccuracy),
            _ => Err(ConversionError::InvalidAccuracy(accuracy)),
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct RequestedSensorConfiguration {
    pub sample_rate: SampleRate,
    pub sensor: VirtualSensorOutput,
}

impl From<&RequestedSensorConfiguration> for bsec_sensor_configuration_t {
    fn from(sensor_configuration: &RequestedSensorConfiguration) -> Self {
        Self {
            sample_rate: sensor_configuration.sample_rate.into(),
            sensor_id: bsec_virtual_sensor_t::from(sensor_configuration.sensor) as u8,
        }
    }
}

#[derive(Clone, Copy, Debug)]
pub struct RequiredSensorSettings {
    sample_rate: f32,
    sensor: PhysicalSensorInput,
}

impl TryFrom<&bsec_sensor_configuration_t> for RequiredSensorSettings {
    type Error = ConversionError;
    fn try_from(
        sensor_configuration: &bsec_sensor_configuration_t,
    ) -> Result<Self, ConversionError> {
        Ok(Self {
            sample_rate: sensor_configuration.sample_rate,
            sensor: PhysicalSensorInput::try_from(sensor_configuration.sensor_id)?,
        })
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum SampleRate {
    Disabled,
    Ulp,
    Continuous,
    Lp,
    UlpMeasurementOnDemand,
}

impl From<SampleRate> for f32 {
    fn from(sample_rate: SampleRate) -> Self {
        f64::from(sample_rate) as f32
    }
}

impl From<SampleRate> for f64 {
    fn from(sample_rate: SampleRate) -> Self {
        use SampleRate::*;
        match sample_rate {
            Disabled => BSEC_SAMPLE_RATE_DISABLED,
            Ulp => BSEC_SAMPLE_RATE_ULP,
            Continuous => BSEC_SAMPLE_RATE_CONTINUOUS,
            Lp => BSEC_SAMPLE_RATE_LP,
            UlpMeasurementOnDemand => BSEC_SAMPLE_RATE_ULP_MEASUREMENT_ON_DEMAND,
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum PhysicalSensorInput {
    Pressure,
    Humidity,
    Temperature,
    GasResistor,
    HeatSource,
    DisableBaselineTracker,
}

impl TryFrom<u8> for PhysicalSensorInput {
    type Error = ConversionError;
    fn try_from(physical_sensor: u8) -> Result<Self, ConversionError> {
        Self::try_from(physical_sensor as u32)
    }
}

impl TryFrom<u32> for PhysicalSensorInput {
    type Error = ConversionError;
    fn try_from(physical_sensor: u32) -> Result<Self, ConversionError> {
        #![allow(non_upper_case_globals)]
        use PhysicalSensorInput::*;
        match physical_sensor {
            bsec_physical_sensor_t_BSEC_INPUT_PRESSURE => Ok(Pressure),
            bsec_physical_sensor_t_BSEC_INPUT_HUMIDITY => Ok(Humidity),
            bsec_physical_sensor_t_BSEC_INPUT_TEMPERATURE => Ok(Temperature),
            bsec_physical_sensor_t_BSEC_INPUT_GASRESISTOR => Ok(GasResistor),
            bsec_physical_sensor_t_BSEC_INPUT_HEATSOURCE => Ok(HeatSource),
            bsec_physical_sensor_t_BSEC_INPUT_DISABLE_BASELINE_TRACKER => {
                Ok(DisableBaselineTracker)
            }
            physical_sensor => Err(ConversionError::InvalidPhysicalSensorId(physical_sensor)),
        }
    }
}

impl From<PhysicalSensorInput> for bsec_physical_sensor_t {
    fn from(physical_sensor: PhysicalSensorInput) -> Self {
        use PhysicalSensorInput::*;
        match physical_sensor {
            Pressure => bsec_physical_sensor_t_BSEC_INPUT_PRESSURE,
            Humidity => bsec_physical_sensor_t_BSEC_INPUT_HUMIDITY,
            Temperature => bsec_physical_sensor_t_BSEC_INPUT_TEMPERATURE,
            GasResistor => bsec_physical_sensor_t_BSEC_INPUT_GASRESISTOR,
            HeatSource => bsec_physical_sensor_t_BSEC_INPUT_HEATSOURCE,
            DisableBaselineTracker => bsec_physical_sensor_t_BSEC_INPUT_DISABLE_BASELINE_TRACKER,
        }
    }
}

impl From<PhysicalSensorInput> for u8 {
    fn from(physical_sensor: PhysicalSensorInput) -> Self {
        bsec_physical_sensor_t::from(physical_sensor) as Self
    }
}

#[derive(Clone, Copy, Debug, Eq, Hash, PartialEq)]
pub enum VirtualSensorOutput {
    Iaq = 0x0001,
    StaticIaq = 0x0002,
    Co2Equivalent = 0x0004,
    BreathVocEquivalent = 0x0008,
    RawTemperature = 0x0010,
    RawPressure = 0x0020,
    RawHumidity = 0x0040,
    RawGas = 0x0080,
    StabilizationStatus = 0x0100,
    RunInStatus = 0x0200,
    SensorHeatCompensatedTemperature = 0x0400,
    SensorHeatCompensatedHumidity = 0x0800,
    DebugCompensatedGas = 0x1000,
    GasPercentage = 0x2000,
}

impl From<VirtualSensorOutput> for bsec_virtual_sensor_t {
    fn from(virtual_sensor: VirtualSensorOutput) -> Self {
        use VirtualSensorOutput::*;
        match virtual_sensor {
            Iaq => bsec_virtual_sensor_t_BSEC_OUTPUT_IAQ,
            StaticIaq => bsec_virtual_sensor_t_BSEC_OUTPUT_STATIC_IAQ,
            Co2Equivalent => bsec_virtual_sensor_t_BSEC_OUTPUT_CO2_EQUIVALENT,
            BreathVocEquivalent => bsec_virtual_sensor_t_BSEC_OUTPUT_BREATH_VOC_EQUIVALENT,
            RawTemperature => bsec_virtual_sensor_t_BSEC_OUTPUT_RAW_TEMPERATURE,
            RawPressure => bsec_virtual_sensor_t_BSEC_OUTPUT_RAW_PRESSURE,
            RawHumidity => bsec_virtual_sensor_t_BSEC_OUTPUT_RAW_HUMIDITY,
            RawGas => bsec_virtual_sensor_t_BSEC_OUTPUT_RAW_GAS,
            StabilizationStatus => bsec_virtual_sensor_t_BSEC_OUTPUT_STABILIZATION_STATUS,
            RunInStatus => bsec_virtual_sensor_t_BSEC_OUTPUT_RUN_IN_STATUS,
            SensorHeatCompensatedTemperature => {
                bsec_virtual_sensor_t_BSEC_OUTPUT_SENSOR_HEAT_COMPENSATED_TEMPERATURE
            }
            SensorHeatCompensatedHumidity => {
                bsec_virtual_sensor_t_BSEC_OUTPUT_SENSOR_HEAT_COMPENSATED_HUMIDITY
            }
            DebugCompensatedGas => bsec_virtual_sensor_t_BSEC_OUTPUT_COMPENSATED_GAS,
            GasPercentage => bsec_virtual_sensor_t_BSEC_OUTPUT_GAS_PERCENTAGE,
        }
    }
}

impl TryFrom<bsec_virtual_sensor_t> for VirtualSensorOutput {
    type Error = ConversionError;
    fn try_from(virtual_sensor: bsec_virtual_sensor_t) -> Result<Self, ConversionError> {
        #![allow(non_upper_case_globals)]
        use VirtualSensorOutput::*;
        match virtual_sensor {
            bsec_virtual_sensor_t_BSEC_OUTPUT_IAQ => Ok(Iaq),
            bsec_virtual_sensor_t_BSEC_OUTPUT_STATIC_IAQ => Ok(StaticIaq),
            bsec_virtual_sensor_t_BSEC_OUTPUT_CO2_EQUIVALENT => Ok(Co2Equivalent),
            bsec_virtual_sensor_t_BSEC_OUTPUT_BREATH_VOC_EQUIVALENT => Ok(BreathVocEquivalent),
            bsec_virtual_sensor_t_BSEC_OUTPUT_RAW_TEMPERATURE => Ok(RawTemperature),
            bsec_virtual_sensor_t_BSEC_OUTPUT_RAW_PRESSURE => Ok(RawPressure),
            bsec_virtual_sensor_t_BSEC_OUTPUT_RAW_HUMIDITY => Ok(RawHumidity),
            bsec_virtual_sensor_t_BSEC_OUTPUT_RAW_GAS => Ok(RawGas),
            bsec_virtual_sensor_t_BSEC_OUTPUT_STABILIZATION_STATUS => Ok(StabilizationStatus),
            bsec_virtual_sensor_t_BSEC_OUTPUT_RUN_IN_STATUS => Ok(RunInStatus),
            bsec_virtual_sensor_t_BSEC_OUTPUT_SENSOR_HEAT_COMPENSATED_TEMPERATURE => {
                Ok(SensorHeatCompensatedTemperature)
            }
            bsec_virtual_sensor_t_BSEC_OUTPUT_SENSOR_HEAT_COMPENSATED_HUMIDITY => {
                Ok(SensorHeatCompensatedHumidity)
            }
            bsec_virtual_sensor_t_BSEC_OUTPUT_COMPENSATED_GAS => Ok(DebugCompensatedGas),
            bsec_virtual_sensor_t_BSEC_OUTPUT_GAS_PERCENTAGE => Ok(GasPercentage),
            _ => Err(ConversionError::InvalidVirtualSensorId(virtual_sensor)),
        }
    }
}

impl TryFrom<u8> for VirtualSensorOutput {
    type Error = ConversionError;
    fn try_from(virtual_sensor: u8) -> Result<Self, ConversionError> {
        Self::try_from(virtual_sensor as bsec_virtual_sensor_t)
    }
}

#[derive(Clone, Debug)]
pub enum Error<E> {
    ArgumentListTooLong,
    BsecAlreadyInUse,
    BsecError(BsecError),
    ConversionError(ConversionError),
    BmeSensorError(E),
}

impl<E> std::fmt::Display for Error<E> {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> Result<(), std::fmt::Error> {
        // TODO
        f.write_fmt(format_args!("Error"))
    }
}

impl<E: std::fmt::Debug> std::error::Error for Error<E> {}

impl<E> From<BsecError> for Error<E> {
    fn from(bsec_error: BsecError) -> Self {
        Self::BsecError(bsec_error)
    }
}

impl<E> From<ConversionError> for Error<E> {
    fn from(conversion_error: ConversionError) -> Self {
        Self::ConversionError(conversion_error)
    }
}

#[derive(Clone, Debug)]
pub enum ConversionError {
    InvalidSampleRate(f64),
    InvalidPhysicalSensorId(bsec_physical_sensor_t),
    InvalidVirtualSensorId(bsec_virtual_sensor_t),
    InvalidAccuracy(u8),
}

type BsecResult = Result<(), BsecError>;

pub trait IntoResult {
    fn into_result(self) -> BsecResult;
}

impl IntoResult for bsec_library_return_t {
    fn into_result(self) -> BsecResult {
        #![allow(non_upper_case_globals)]
        match self {
            bsec_library_return_t_BSEC_OK => Ok(()),
            error_code => Err(BsecError::from(error_code)),
        }
    }
}

#[derive(Clone, Debug)]
pub enum BsecError {
    DoStepsInvalidInput,
    DoStepsValueLimits,
    DoStepsDuplicateInput,
    DoStepsNoOutputsReturnable,
    DoStepsExcessOutputs,
    DoStepsTsIntraDiffOutOfRange,
    UpdateSubscriptionWrongDataRate,
    UpdateSubscriptionSampleRateLimits,
    UpdateSubscriptionDuplicateGate,
    UpdateSubscriptionInvalidSampleRate,
    UpdateSubscriptionGateCountExceedsArray,
    UpdateSubscriptionSampleIntervalIntegerMult,
    UpdateSubscriptionMultGaaSamplInterval,
    UpdateSubscriptionHighHeaterOnDuration,
    UpdateSubscriptionUnkownOutputGate,
    UpdateSubscriptionModeInNonUlp,
    UpdateSubscriptionSubscribedOutputGates,
    ParseSectionExceedsWorkBuffer,
    ConfigFail,
    ConfigVersionMismatch,
    ConfigFeatureMismatch,
    ConfigCrcMismatch,
    ConfigEmpty,
    ConfigInsufficientWorkBuffer,
    ConfigInvalidStringSize,
    ConfigInsufficientBuffer,
    SetInvalidChannelIdentifier,
    SetInvalidLength,
    SensorControlCallTimingViolation,
    SensorControlModeExceedsUlpTimelimit,
    SensorControlModeInsufficientWaitTime,
    Unknown(bsec_library_return_t),
}

impl From<bsec_library_return_t> for BsecError {
    fn from(return_code: bsec_library_return_t) -> Self {
        #![allow(non_upper_case_globals)]
        use BsecError::*;
        match return_code {
            bsec_library_return_t_BSEC_E_DOSTEPS_INVALIDINPUT => DoStepsInvalidInput,
            bsec_library_return_t_BSEC_E_DOSTEPS_VALUELIMITS => DoStepsValueLimits,
            bsec_library_return_t_BSEC_E_DOSTEPS_DUPLICATEINPUT => DoStepsDuplicateInput,
            bsec_library_return_t_BSEC_I_DOSTEPS_NOOUTPUTSRETURNABLE => DoStepsNoOutputsReturnable,
            bsec_library_return_t_BSEC_W_DOSTEPS_EXCESSOUTPUTS => DoStepsExcessOutputs,
            bsec_library_return_t_BSEC_W_DOSTEPS_TSINTRADIFFOUTOFRANGE => {
                DoStepsTsIntraDiffOutOfRange
            }
            bsec_library_return_t_BSEC_E_SU_WRONGDATARATE => UpdateSubscriptionWrongDataRate,
            bsec_library_return_t_BSEC_E_SU_SAMPLERATELIMITS => UpdateSubscriptionSampleRateLimits,
            bsec_library_return_t_BSEC_E_SU_DUPLICATEGATE => UpdateSubscriptionDuplicateGate,
            bsec_library_return_t_BSEC_E_SU_INVALIDSAMPLERATE => {
                UpdateSubscriptionInvalidSampleRate
            }
            bsec_library_return_t_BSEC_E_SU_GATECOUNTEXCEEDSARRAY => {
                UpdateSubscriptionGateCountExceedsArray
            }
            bsec_library_return_t_BSEC_E_SU_SAMPLINTVLINTEGERMULT => {
                UpdateSubscriptionSampleIntervalIntegerMult
            }
            bsec_library_return_t_BSEC_E_SU_MULTGASSAMPLINTVL => {
                UpdateSubscriptionMultGaaSamplInterval
            }
            bsec_library_return_t_BSEC_E_SU_HIGHHEATERONDURATION => {
                UpdateSubscriptionHighHeaterOnDuration
            }
            bsec_library_return_t_BSEC_W_SU_UNKNOWNOUTPUTGATE => UpdateSubscriptionUnkownOutputGate,
            bsec_library_return_t_BSEC_W_SU_MODINNOULP => UpdateSubscriptionModeInNonUlp,
            bsec_library_return_t_BSEC_I_SU_SUBSCRIBEDOUTPUTGATES => {
                UpdateSubscriptionSubscribedOutputGates
            }
            bsec_library_return_t_BSEC_E_PARSE_SECTIONEXCEEDSWORKBUFFER => {
                ParseSectionExceedsWorkBuffer
            }
            bsec_library_return_t_BSEC_E_CONFIG_FAIL => ConfigFail,
            bsec_library_return_t_BSEC_E_CONFIG_VERSIONMISMATCH => ConfigVersionMismatch,
            bsec_library_return_t_BSEC_E_CONFIG_FEATUREMISMATCH => ConfigFeatureMismatch,
            bsec_library_return_t_BSEC_E_CONFIG_CRCMISMATCH => ConfigCrcMismatch,
            bsec_library_return_t_BSEC_E_CONFIG_EMPTY => ConfigEmpty,
            bsec_library_return_t_BSEC_E_CONFIG_INSUFFICIENTWORKBUFFER => {
                ConfigInsufficientWorkBuffer
            }
            bsec_library_return_t_BSEC_E_CONFIG_INVALIDSTRINGSIZE => ConfigInvalidStringSize,
            bsec_library_return_t_BSEC_E_CONFIG_INSUFFICIENTBUFFER => ConfigInsufficientBuffer,
            bsec_library_return_t_BSEC_E_SET_INVALIDCHANNELIDENTIFIER => {
                SetInvalidChannelIdentifier
            }
            bsec_library_return_t_BSEC_E_SET_INVALIDLENGTH => SetInvalidLength,
            bsec_library_return_t_BSEC_W_SC_CALL_TIMING_VIOLATION => {
                SensorControlCallTimingViolation
            }
            bsec_library_return_t_BSEC_W_SC_MODEXCEEDULPTIMELIMIT => {
                SensorControlModeExceedsUlpTimelimit
            }
            bsec_library_return_t_BSEC_W_SC_MODINSUFFICIENTWAITTIME => {
                SensorControlModeInsufficientWaitTime
            }
            return_code => Unknown(return_code),
        }
    }
}

pub mod ffi {
    #![allow(non_camel_case_types)]
    #![allow(non_upper_case_globals)]

    include!(concat!(env!("OUT_DIR"), "/bsec_bindings.rs"));
}

#[cfg(test)]
mod tests {
    use super::*;
    use serial_test::serial;
    use std::cell::RefCell;
    use std::collections::HashMap;

    struct FakeBmeSensor {
        measurement: nb::Result<Vec<BmeOutput>, ()>,
    }
    impl FakeBmeSensor {
        fn new(measurement: nb::Result<Vec<BmeOutput>, ()>) -> Self {
            Self { measurement }
        }
    }
    impl Default for FakeBmeSensor {
        fn default() -> Self {
            Self::new(Ok(vec![]))
        }
    }
    impl BmeSensor for FakeBmeSensor {
        type Error = ();
        fn start_measurement(
            &mut self,
            _: &BmeSettingsHandle<'_>,
        ) -> Result<std::time::Duration, ()> {
            Ok(std::time::Duration::new(0, 0))
        }
        fn get_measurement(&mut self) -> nb::Result<Vec<BmeOutput>, ()> {
            self.measurement.clone()
        }
    }

    #[derive(Default)]
    struct FakeTime {
        timestamp_ns: RefCell<i64>,
    }
    impl Time for FakeTime {
        fn timestamp_ns(&self) -> i64 {
            *self.timestamp_ns.borrow_mut() += 1;
            *self.timestamp_ns.borrow()
        }
    }
    impl FakeTime {
        fn advance_by(&self, duration: Duration) {
            *self.timestamp_ns.borrow_mut() += duration.as_nanos() as i64;
        }
    }

    #[test]
    #[serial]
    fn cannot_create_mulitple_bsec_at_the_same_time() {
        let time = FakeTime::default();
        let first = Bsec::init(FakeBmeSensor::default(), &time).unwrap();
        assert!(Bsec::init(FakeBmeSensor::default(), &time).is_err());
        drop(first);
        let _another = Bsec::init(FakeBmeSensor::default(), &time).unwrap();
    }

    #[test]
    #[serial]
    fn basic_bsec_operation_smoke_test() {
        let time = FakeTime::default();
        let sensor = FakeBmeSensor::new(Ok(vec![
            BmeOutput {
                sensor: PhysicalSensorInput::Temperature,
                signal: 22.,
            },
            BmeOutput {
                sensor: PhysicalSensorInput::Humidity,
                signal: 40.,
            },
            BmeOutput {
                sensor: PhysicalSensorInput::Pressure,
                signal: 1000.,
            },
            BmeOutput {
                sensor: PhysicalSensorInput::GasResistor,
                signal: 6000.,
            },
        ]));
        let mut bsec = Bsec::init(sensor, &time).unwrap();
        bsec.update_subscription(&[
            RequestedSensorConfiguration {
                sample_rate: SampleRate::Lp,
                sensor: VirtualSensorOutput::RawTemperature,
            },
            RequestedSensorConfiguration {
                sample_rate: SampleRate::Lp,
                sensor: VirtualSensorOutput::RawHumidity,
            },
            RequestedSensorConfiguration {
                sample_rate: SampleRate::Lp,
                sensor: VirtualSensorOutput::RawPressure,
            },
            RequestedSensorConfiguration {
                sample_rate: SampleRate::Lp,
                sensor: VirtualSensorOutput::RawGas,
            },
        ])
        .unwrap();

        time.advance_by(bsec.start_next_measurement().unwrap());
        let outputs = bsec.process_last_measurement().unwrap();
        assert!(bsec.next_measurement() >= 3_000_000_000);

        let signals: HashMap<VirtualSensorOutput, &OutputSignal> =
            outputs.iter().map(|s| (s.sensor, s)).collect();
        assert!(
            (signals
                .get(&VirtualSensorOutput::RawTemperature)
                .unwrap()
                .signal
                - 22.)
                .abs()
                < f64::EPSILON
        );
        assert!(
            (signals
                .get(&VirtualSensorOutput::RawHumidity)
                .unwrap()
                .signal
                - 40.)
                .abs()
                < f64::EPSILON
        );
        assert!(
            (signals
                .get(&VirtualSensorOutput::RawPressure)
                .unwrap()
                .signal
                - 1000.)
                .abs()
                < f64::EPSILON
        );
        assert!(
            (signals.get(&VirtualSensorOutput::RawGas).unwrap().signal - 6000.).abs()
                < f64::EPSILON
        );
    }

    #[test]
    #[serial]
    fn roundtrip_state_smoke_test() {
        let time = FakeTime::default();
        let sensor = FakeBmeSensor::default();
        let mut bsec = Bsec::init(sensor, &time).unwrap();
        let state = bsec.get_state().unwrap();
        bsec.set_state(&state).unwrap();
    }

    #[test]
    #[serial]
    fn configuration_roundtrip_smoke_test() {
        let time = FakeTime::default();
        let sensor = FakeBmeSensor::default();
        let mut bsec = Bsec::init(sensor, &time).unwrap();
        let config = include_bytes!("../bsec/config/generic_33v_3s_4d/bsec_iaq.config");
        let config = &config[4..]; // First 4 bytes give config length
        bsec.set_configuration(config).unwrap();
        assert_eq!(bsec.get_configuration().unwrap(), config);
    }

    #[test]
    fn get_version_smoke_test() {
        let version = get_version().unwrap();
        assert!(version.0 == 1);
        assert!(version.1 >= 4);
        assert!(version.1 > 4 || version.2 >= 8);
    }

    #[test]
    #[serial]
    fn reset_output_smoke_test() {
        let time = FakeTime::default();
        let sensor = FakeBmeSensor::default();
        let mut bsec = Bsec::init(sensor, &time).unwrap();
        bsec.reset_output(VirtualSensorOutput::Iaq).unwrap();
    }
}
