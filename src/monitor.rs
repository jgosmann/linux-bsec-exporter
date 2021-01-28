use super::bsec::{self, BmeSensor, Bsec, OutputSignal, Time};
use anyhow::Result;
use nb::block;
use std::future::{self, Future};
use std::sync::Arc;
use tokio::sync::{oneshot, watch};
use tokio::task::JoinHandle;
use tokio::time::Duration;

pub trait PersistState {
    type Error;
    fn load_state(&mut self) -> Result<Option<Vec<u8>>, Self::Error>;
    fn save_state(&mut self, state: &[u8]) -> Result<(), Self::Error>;
}

pub trait Sleep {
    type SleepFuture: Future;
    fn sleep(&self, duration: Duration) -> Self::SleepFuture;
}

pub struct Monitor<S, P, T>
where
    S: BmeSensor + 'static,
    P: PersistState + 'static,
    T: Time + Sleep + 'static,
    P::Error: std::error::Error + Send + Sync + 'static,
    S::Error: std::error::Error + Send + Sync + 'static,
{
    pub current: watch::Receiver<Vec<OutputSignal>>,
    request_shutdown: oneshot::Sender<()>,
    join_handle: JoinHandle<Result<(Bsec<S, T, Arc<T>>, P)>>,
}

impl<S, P, T> Monitor<S, P, T>
where
    S: BmeSensor + 'static,
    P: PersistState + 'static,
    T: Time + Sleep + 'static,
    P::Error: std::error::Error + Send + Sync + 'static,
    S::Error: std::error::Error + Send + Sync + 'static,
{
    pub async fn start(bsec: Bsec<S, T, Arc<T>>, persistence: P, time: Arc<T>) -> Result<Self> {
        let mut bsec = bsec;
        let (set_current, current) =
            watch::channel(Self::next_measurement(&mut bsec, time.clone()).await?);
        let (request_shutdown, shutdown_requested) = oneshot::channel();
        let join_handle = tokio::task::spawn_local(Self::monitoring_loop(
            bsec,
            persistence,
            time,
            set_current,
            shutdown_requested,
        ));
        Ok(Self {
            current,
            request_shutdown,
            join_handle,
        })
    }

    async fn monitoring_loop(
        bsec: Bsec<S, T, Arc<T>>,
        persistence: P,
        time: Arc<T>,
        set_current: watch::Sender<Vec<OutputSignal>>,
        shutdown_requested: oneshot::Receiver<()>,
    ) -> Result<(Bsec<S, T, Arc<T>>, P)> {
        let mut bsec = bsec;
        let mut persistence = persistence;
        let mut shutdown_requested = shutdown_requested;
        let mut last_state_save = time.timestamp_ns();

        if let Some(state) = persistence.load_state()? {
            bsec.set_state(&state)?;
        }

        while shutdown_requested.try_recv().is_err() {
            set_current.send(Self::next_measurement(&mut bsec, time.clone()).await?)?;
            if time.timestamp_ns() - last_state_save >= 60_000_000_000 {
                last_state_save = time.timestamp_ns();
                persistence.save_state(&bsec.get_state()?)?;
            }
            tokio::task::yield_now().await;
        }

        persistence.save_state(&bsec.get_state()?)?;

        Ok((bsec, persistence))
    }

    async fn next_measurement(
        bsec: &mut Bsec<S, T, Arc<T>>,
        time: Arc<T>,
    ) -> Result<Vec<OutputSignal>, bsec::Error<S::Error>> {
        let sleep_duration = bsec.next_measurement() - time.timestamp_ns();
        if sleep_duration > 0 {
            time.sleep(Duration::from_nanos(sleep_duration as u64))
                .await;
        }
        let duration = block!(bsec.start_next_measurement())?;
        time.sleep(duration).await;
        block!(bsec.process_last_measurement())
    }

    async fn stop(self) -> Result<(Bsec<S, T, Arc<T>>, P), anyhow::Error> {
        let _ = self.request_shutdown.send(());
        self.join_handle.await?
    }
}

#[cfg(test)]
mod tests {
    use super::bsec::tests::{FakeBmeSensor, FakeTime};
    use super::bsec::{
        BmeOutput, PhysicalSensorInput, RequestedSensorConfiguration, SampleRate,
        VirtualSensorOutput,
    };
    use super::*;
    use serial_test::serial;
    use std::cell::RefCell;
    use std::future::Ready;

    impl Sleep for FakeTime {
        type SleepFuture = Ready<()>;
        fn sleep(&self, duration: Duration) -> Self::SleepFuture {
            self.advance_by(duration);
            future::ready(())
        }
    }

    #[derive(Default)]
    struct MockPersistState {
        pub state: Arc<RefCell<Option<Vec<u8>>>>,
    }
    impl PersistState for MockPersistState {
        type Error = std::convert::Infallible;

        fn load_state(&mut self) -> Result<Option<Vec<u8>>, Self::Error> {
            Ok(self.state.borrow().clone())
        }

        fn save_state(&mut self, state: &[u8]) -> Result<(), Self::Error> {
            *self.state.borrow_mut() = Some(Vec::from(state));
            Ok(())
        }
    }

    fn create_minimal_subscribed_bsec<T: Time>(time: Arc<T>) -> Bsec<FakeBmeSensor, T, Arc<T>> {
        let bme = FakeBmeSensor::new(Ok(vec![BmeOutput {
            sensor: PhysicalSensorInput::Temperature,
            signal: 22.,
        }]));
        let mut bsec = Bsec::init(bme, time).unwrap();
        bsec.update_subscription(&[RequestedSensorConfiguration {
            sample_rate: SampleRate::Continuous,
            sensor: VirtualSensorOutput::RawTemperature,
        }])
        .unwrap();
        bsec
    }

    #[tokio::test]
    #[serial]
    async fn smoke_test() {
        let time = Arc::new(FakeTime::default());
        let bme = FakeBmeSensor::new(Ok(vec![BmeOutput {
            sensor: PhysicalSensorInput::Temperature,
            signal: 22.,
        }]));
        let mut bsec = Bsec::init(bme, time.clone()).unwrap();
        bsec.update_subscription(&[RequestedSensorConfiguration {
            sample_rate: SampleRate::Continuous,
            sensor: VirtualSensorOutput::RawTemperature,
        }])
        .unwrap();

        let local = tokio::task::LocalSet::new();

        local
            .run_until(async move {
                let mut monitor = Monitor::start(bsec, MockPersistState::default(), time.clone())
                    .await
                    .unwrap();
                {
                    let outputs = monitor.current.borrow();
                    assert_eq!(outputs.len(), 1);
                    assert_eq!(outputs[0].sensor, VirtualSensorOutput::RawTemperature);
                    assert!((outputs[0].signal - 22.) < f64::EPSILON);
                }

                let _ = monitor.current.changed().await.and_then(|_| {
                    let outputs = monitor.current.borrow();
                    assert_eq!(outputs.len(), 1);
                    assert_eq!(outputs[0].sensor, VirtualSensorOutput::RawTemperature);
                    assert!((outputs[0].signal - 22.) < f64::EPSILON);
                    Ok(())
                });

                monitor.stop().await.unwrap();
            })
            .await;
    }

    #[tokio::test]
    #[serial]
    async fn loads_and_persists_state() {
        let time = Arc::new(FakeTime::default());
        let bsec = create_minimal_subscribed_bsec(time.clone());

        let state = Arc::new(RefCell::new(Some(bsec.get_state().unwrap())));
        let persist_state = MockPersistState {
            state: state.clone(),
        };

        let local = tokio::task::LocalSet::new();

        local
            .run_until(async move {
                let monitor = Monitor::start(bsec, persist_state, time.clone())
                    .await
                    .unwrap();
                *state.borrow_mut() = None;
                let (bsec, _) = monitor.stop().await.unwrap();
                assert_eq!(*state.borrow(), Some(bsec.get_state().unwrap()));
            })
            .await;
    }

    #[tokio::test]
    #[serial]
    async fn autosaves_state() {
        let time = Arc::new(FakeTime::default());
        let bsec = create_minimal_subscribed_bsec(time.clone());

        let state = Arc::new(RefCell::new(None));
        let persist_state = MockPersistState {
            state: state.clone(),
        };

        let local = tokio::task::LocalSet::new();

        local
            .run_until(async move {
                let monitor = Monitor::start(bsec, persist_state, time.clone())
                    .await
                    .unwrap();

                for _ in 0..70 {
                    time.sleep(Duration::from_secs(1)).await;
                    tokio::task::yield_now().await
                }

                assert!(state.borrow().is_some());
                monitor.stop().await.unwrap();
            })
            .await;
    }
}
