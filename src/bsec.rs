use self::ffi::*;
use std::convert::{From, Into, TryInto};
use std::sync::atomic::{AtomicBool, Ordering};

static BSEC_IN_USE: AtomicBool = AtomicBool::new(false);

pub struct Bsec {
    _disallow_creation_by_member_initialization: (),
}

impl Bsec {
    fn init() -> Result<Self, Error> {
        if !BSEC_IN_USE.compare_and_swap(false, true, Ordering::SeqCst) {
            unsafe {
                bsec_init().into_result()?;
            }
            Ok(Self {
                _disallow_creation_by_member_initialization: (),
            })
        } else {
            Err(Error::BsecAlreadyInUse)
        }
    }

    fn update_subscription(
        &mut self,
        requested_outputs: &Vec<SensorConfiguration>,
    ) -> Result<(), Error> {
        let requested_outputs: Vec<bsec_sensor_configuration_t> =
            requested_outputs.iter().map(Into::into).collect();
        let mut required_sensor_settings: [bsec_sensor_configuration_t; 0] = [];
        let mut n_required_sensor_settings = 0;
        unsafe {
            bsec_update_subscription(
                requested_outputs.as_ptr(),
                requested_outputs
                    .len()
                    .try_into()
                    .or(Err(Error::ArgumentListTooLong))?,
                required_sensor_settings.as_mut_ptr(),
                &mut n_required_sensor_settings,
            )
            .into_result()?
        }
        Ok(())
    }
}

impl Drop for Bsec {
    fn drop(&mut self) {
        BSEC_IN_USE.store(false, Ordering::SeqCst);
    }
}

pub struct SensorConfiguration {
    sample_rate: SampleRate,
    sensor: VirtualSensorOutput,
}

impl From<&SensorConfiguration> for bsec_sensor_configuration_t {
    fn from(sensor_configuration: &SensorConfiguration) -> Self {
        bsec_sensor_configuration_t {
            sample_rate: sensor_configuration.sample_rate.into(),
            sensor_id: sensor_configuration.sensor.into(),
        }
    }
}

#[derive(Clone, Copy, Debug)]
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

#[derive(Clone, Copy, Debug)]
pub enum VirtualSensorOutput {
    Iaq,
    StaticIaq,
    Co2Equivalent,
    BreathVocEquivalent,
    RawTemperature,
    RawPressure,
    RawHumidity,
    RawGas,
    StabilizationStatus,
    RunInStatus,
    SensorHeatCompensatedTemperature,
    SensorHeatCompensatedHumidity,
    DebugCompensatedGas,
    GasPercentage,
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

impl From<VirtualSensorOutput> for u8 {
    fn from(virtual_sensor: VirtualSensorOutput) -> Self {
        bsec_virtual_sensor_t::from(virtual_sensor) as u8
    }
}

#[derive(Clone, Debug)]
pub enum Error {
    ArgumentListTooLong,
    BsecAlreadyInUse,
    BsecError(BsecError),
}

impl From<BsecError> for Error {
    fn from(bsec_error: BsecError) -> Self {
        Self::BsecError(bsec_error)
    }
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

    #[test]
    fn cannot_create_mulitple_bsec_at_the_same_time() {
        let first = Bsec::init().unwrap();
        assert!(Bsec::init().is_err());
        drop(first);
        let _another = Bsec::init().unwrap();
    }
}
