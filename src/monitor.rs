use super::bsec::{self, BmeSensor, Bsec, OutputSignal, Time};
use anyhow::Result;
use nb::block;
use std::sync::Arc;
use tokio::sync::{oneshot, watch};
use tokio::task::JoinHandle;
use tokio::time::{sleep, Duration};

pub trait PersistState {
    type Error;
    fn load_state(&mut self) -> Result<Option<Vec<u8>>, Self::Error>;
    fn save_state(&mut self, state: &[u8]) -> Result<(), Self::Error>;
}

pub struct Monitor<S, P, T>
where
    S: BmeSensor + 'static,
    P: PersistState + 'static,
    T: Time + 'static,
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
    T: Time + 'static,
    P::Error: std::error::Error + Send + Sync + 'static,
    S::Error: std::error::Error + Send + Sync + 'static,
{
    pub async fn start(bsec: Bsec<S, T, Arc<T>>, persistence: P, time: Arc<T>) -> Result<Self>
    where
        S: BmeSensor + 'static,
        P: PersistState + 'static,
        T: Time + 'static,
        P::Error: std::error::Error + Send + Sync + 'static,
        S::Error: std::error::Error + Send + Sync + 'static,
    {
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
    ) -> Result<(Bsec<S, T, Arc<T>>, P)>
    where
        S: BmeSensor + 'static,
        P: PersistState + 'static,
        T: Time,
        P::Error: std::error::Error + Send + Sync + 'static,
        S::Error: std::error::Error + Send + Sync + 'static,
    {
        let mut bsec = bsec;
        let mut persistence = persistence;
        let mut shutdown_requested = shutdown_requested;

        if let Some(state) = persistence.load_state()? {
            bsec.set_state(&state)?;
        }

        while shutdown_requested.try_recv().is_err() {
            set_current.send(Self::next_measurement(&mut bsec, time.clone()).await?)?;
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
            sleep(Duration::from_nanos(sleep_duration as u64)).await;
        }
        let duration = block!(bsec.start_next_measurement())?;
        sleep(duration).await;
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
    use std::time::Instant;

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

    #[tokio::test]
    #[serial]
    async fn smoke_test() {
        let time = Arc::new(TimeAlive::default());
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

                monitor.current.changed().await.unwrap();
                {
                    let outputs = monitor.current.borrow();
                    assert_eq!(outputs.len(), 1);
                    assert_eq!(outputs[0].sensor, VirtualSensorOutput::RawTemperature);
                    assert!((outputs[0].signal - 22.) < f64::EPSILON);
                }

                monitor.stop().await.unwrap();
            })
            .await;
    }

    #[tokio::test]
    #[serial]
    async fn loads_and_persists_state() {
        let time = Arc::new(TimeAlive::default());
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
}
