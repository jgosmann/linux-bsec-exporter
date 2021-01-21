use self::ffi::*;
use std::convert::From;
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
}

impl Drop for Bsec {
    fn drop(&mut self) {
        BSEC_IN_USE.store(false, Ordering::SeqCst);
    }
}

#[derive(Debug)]
pub enum Error {
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

#[derive(Debug, thiserror::Error)]
pub enum BsecError {
    #[error(
        "Input (physical) sensor id passed to bsec_do_steps() is not in \
        the valid range or not valid for requested virtual sensor."
    )]
    DoStepsInvalidInput,
    #[error(
        "Value of input (physical) sensor signal passed to bsec_do_steps() is \
        not in the valid range."
    )]
    DoStepsValueLimits,
    #[error("Duplicate input (physical) sensor ids passed as input to bsec_do_steps().")]
    DoStepsDuplicateInput,
    #[error(
        "No memory allocated to hold return values from bsec_do_steps(), i.e., \
        n_outputs == 0."
    )]
    DoStepsNoOutputsReturnable,
    #[error(
        "Not enough memory allocated to hold return values from \
        bsec_do_steps(), i.e., n_outputs < maximum number of requested output \
        (virtual) sensors."
    )]
    DoStepsExcessOutputs,
    #[error("Duplicate timestamps passed to bsec_do_steps().")]
    DoStepsTsIntraDiffOutOfRange,
    #[error(
        "The sample_rate of the requested output (virtual) sensor passed to \
        bsec_update_subscription() is zero."
    )]
    UpdateSubscriptionWrongDataRate,
    #[error(
        "The sample_rate of the requested output (virtual) sensor passed to \
        bsec_update_subscription() does not match with the sampling rate \
        allowed for that sensor."
    )]
    UpdateSubscriptionSampleRateLimits,
    #[error(
        "Duplicate output (virtual) sensor ids requested through \
        bsec_update_subscription()."
    )]
    UpdateSubscriptionDuplicateGate,
    #[error(
        "The sample_rate of the requested output (virtual) sensor passed to \
        bsec_update_subscription() does not fall within the global minimum and \
        maximum sampling rates."
    )]
    UpdateSubscriptionInvalidSampleRate,
    #[error(
        "Not enough memory allocated to hold returned input (physical) sensor \
        data from bsec_update_subscription(), i.e., \
        n_required_sensor_settings < #BSEC_MAX_PHYSICAL_SENSOR."
    )]
    UpdateSubscriptionGateCountExceedsArray,
    #[error(
        "The sample_rate of the requested output (virtual) sensor passed to \
        bsec_update_subscription() is not correct."
    )]
    UpdateSubscriptionSampleIntervalIntegerMult,
    #[error(
        "The sample_rate of the requested output (virtual), which requires the \
        gas sensor, is not equal to the sample_rate that the gas sensor is \
        being operated."
    )]
    UpdateSubscriptionMultGaaSamplInterval,
    #[error(
        "The duration of one measurement is longer than the requested sampling \
        interval."
    )]
    UpdateSubscriptionHighHeaterOnDuration,
    #[error(
        "Output (virtual) sensor id passed to bsec_update_subscription() is \
        not in the valid range; e.g., n_requested_virtual_sensors > actual \
        number of output (virtual) sensors requested."
    )]
    UpdateSubscriptionUnkownOutputGate,
    #[error("ULP plus can not be requested in non-ulp mode.")]
    UpdateSubscriptionModeInNonUlp,
    #[error(
        "No output (virtual) sensor data were requested \
        via bsec_update_subscription()."
    )]
    UpdateSubscriptionSubscribedOutputGates,
    #[error(
        "n_work_buffer_size passed to bsec_set_[configuration/state]() not \
        sufficient."
    )]
    ParseSectionExceedsWorkBuffer,
    #[error("Configuration failed.")]
    ConfigFail,
    #[error(
        "Version encoded in serialized_[settings/state] passed to \
        bsec_set_[configuration/state]() does not match with current version."
    )]
    ConfigVersionMismatch,
    #[error(
        "Enabled features encoded in serialized_[settings/state] passed to \
        bsec_set_[configuration/state]() does not match with current library \
        implementation."
    )]
    ConfigFeatureMismatch,
    #[error(
        "serialized_[settings/state] passed to \
        bsec_set_[configuration/state]() is corrupted."
    )]
    ConfigCrcMismatch,
    #[error(
        "n_serialized_[settings/state] passed to \
        bsec_set_[configuration/state]() is to short to be valid."
    )]
    ConfigEmpty,
    #[error("Provided work_buffer is not large enough to hold the desired string.")]
    ConfigInsufficientWorkBuffer,
    #[error(
        "String size encoded in configuration/state strings passed to \
        bsec_set_[configuration/state]() does not match with the actual string \
        size n_serialized_[settings/state] passed to these functions."
    )]
    ConfigInvalidStringSize,
    #[error("String buffer insufficient to hold serialized data from BSEC library.")]
    ConfigInsufficientBuffer,
    #[error(
        "Internal error code, size of work buffer in setConfig must be set to \
        BSEC_MAX_WORKBUFFER_SIZE."
    )]
    SetInvalidChannelIdentifier,
    #[error("Internal error code.")]
    SetInvalidLength,
    #[error(
        "Difference between actual and defined sampling intervals of \
        bsec_sensor_control() greater than allowed."
    )]
    SensorControlCallTimingViolation,
    #[error(
        "ULP plus is not allowed because an ULP measurement just took or will \
        take place"
    )]
    SensorControlModeExceedsUlpTimelimit,
    #[error(
        "ULP plus is not allowed because not sufficient time passed since last \
        ULP plus."
    )]
    SensorControlModeInsufficientWaitTime,
    #[error("Unknown error.")]
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
