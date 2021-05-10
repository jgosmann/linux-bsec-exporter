use anyhow::Result;
use bsec::{self, bme::BmeSensor, clock::Clock, Bsec};
use nb::block;
use std::future::Future;
use std::sync::Arc;
use tokio::sync::{oneshot, watch};
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

pub struct BsecReceiver {
    pub current: watch::Receiver<Option<Vec<bsec::Output>>>,
    pub initiate_shutdown: oneshot::Sender<()>,
}

pub struct BsecSender<S, P, C>
where
    S: BmeSensor + 'static,
    P: PersistState + 'static,
    C: Clock + Sleep + 'static,
{
    sender: watch::Sender<Option<Vec<bsec::Output>>>,
    shutdown_request_receiver: oneshot::Receiver<()>,
    bsec: Bsec<S, C, Arc<C>>,
    persistence: P,
    clock: Arc<C>,
}

impl<S, P, C> BsecSender<S, P, C>
where
    S: BmeSensor + 'static,
    P: PersistState + 'static,
    C: Clock + Sleep + 'static,
    P::Error: std::error::Error + Send + Sync + 'static,
    S::Error: std::fmt::Debug + Send + Sync + 'static,
{
    pub async fn monitoring_loop(mut self) -> Result<(Bsec<S, C, Arc<C>>, P)> {
        let mut last_state_save = self.clock.timestamp_ns();

        if let Some(state) = self.persistence.load_state()? {
            self.bsec.set_state(&state)?;
        }

        while self.shutdown_request_receiver.try_recv().is_err() {
            self.sender.send(Some(
                Self::next_measurement(&mut self.bsec, self.clock.clone()).await?,
            ))?;
            if self.clock.timestamp_ns() - last_state_save >= 60_000_000_000 {
                last_state_save = self.clock.timestamp_ns();
                self.persistence.save_state(&self.bsec.get_state()?)?;
            }
            tokio::task::yield_now().await;
        }

        self.persistence.save_state(&self.bsec.get_state()?)?;

        Ok((self.bsec, self.persistence))
    }

    async fn next_measurement(
        bsec: &mut Bsec<S, C, Arc<C>>,
        time: Arc<C>,
    ) -> Result<Vec<bsec::Output>, bsec::error::Error<S::Error>> {
        let sleep_duration = bsec.next_measurement() - time.timestamp_ns();
        if sleep_duration > 0 {
            time.sleep(Duration::from_nanos(sleep_duration as u64))
                .await;
        }
        let duration = block!(bsec.start_next_measurement())?;
        time.sleep(duration).await;
        block!(bsec.process_last_measurement())
    }
}

pub fn bsec_monitor<S, P, C>(
    bsec: Bsec<S, C, Arc<C>>,
    persistence: P,
    clock: Arc<C>,
) -> (BsecSender<S, P, C>, BsecReceiver)
where
    S: BmeSensor + 'static,
    P: PersistState + 'static,
    C: Clock + Sleep + 'static,
    P::Error: std::error::Error + Send + Sync + 'static,
    S::Error: std::fmt::Debug + Send + Sync + 'static,
{
    let (sender, receiver) = watch::channel(None);
    let (initiate_shutdown, shutdown_request_receiver) = oneshot::channel();
    (
        BsecSender {
            sender,
            shutdown_request_receiver,
            bsec,
            persistence,
            clock,
        },
        BsecReceiver {
            current: receiver,
            initiate_shutdown,
        },
    )
}

#[cfg(test)]
mod tests {
    use super::*;
    use bsec::bme::test_support::FakeBmeSensor;
    use bsec::clock::test_support::FakeClock;
    use serial_test::serial;
    use std::future::{self, Ready};

    impl Sleep for FakeClock {
        type SleepFuture = Ready<()>;

        fn sleep(&self, duration: Duration) -> Self::SleepFuture {
            self.advance_by(duration);
            future::ready(())
        }
    }

    #[derive(Default)]
    struct MockPersistState {
        pub state: Arc<std::sync::RwLock<Option<Vec<u8>>>>,
    }

    impl PersistState for MockPersistState {
        type Error = std::convert::Infallible;

        fn load_state(&mut self) -> Result<Option<Vec<u8>>, Self::Error> {
            Ok(self.state.read().unwrap().clone())
        }

        fn save_state(&mut self, state: &[u8]) -> Result<(), Self::Error> {
            *self.state.write().unwrap() = Some(Vec::from(state));
            Ok(())
        }
    }

    fn create_minimal_subscribed_bsec<C: Clock>(time: Arc<C>) -> Bsec<FakeBmeSensor, C, Arc<C>> {
        let bme = FakeBmeSensor::new(Ok(vec![bsec::Input {
            sensor: bsec::InputKind::Temperature,
            signal: 22.,
        }]));
        let mut bsec = Bsec::init(bme, time).unwrap();
        bsec.update_subscription(&[bsec::SubscriptionRequest {
            sample_rate: bsec::SampleRate::Continuous,
            sensor: bsec::OutputKind::RawTemperature,
        }])
        .unwrap();
        bsec
    }

    #[tokio::test]
    #[serial]
    async fn smoke_test() {
        let clock = Arc::new(FakeClock::new());
        let bme = FakeBmeSensor::new(Ok(vec![bsec::Input {
            sensor: bsec::InputKind::Temperature,
            signal: 22.,
        }]));
        let mut bsec = Bsec::init(bme, clock.clone()).unwrap();
        bsec.update_subscription(&[bsec::SubscriptionRequest {
            sample_rate: bsec::SampleRate::Continuous,
            sensor: bsec::OutputKind::RawTemperature,
        }])
        .unwrap();

        let (monitor, mut rx) =
            bsec_monitor(bsec, MockPersistState::default(), clock.clone());
        {
            assert_eq!(*rx.current.borrow(), None);
        }

        let join_handle = tokio::task::spawn(monitor.monitoring_loop());
        let _ = rx.current.changed().await.and_then(|_| {
            let borrow = rx.current.borrow();
            let outputs = borrow.as_deref().unwrap();
            assert_eq!(outputs.len(), 1);
            assert_eq!(outputs[0].sensor, bsec::OutputKind::RawTemperature);
            assert!((outputs[0].signal - 22.) < f64::EPSILON);
            Ok(())
        });

        rx.initiate_shutdown.send(()).unwrap();
        join_handle.await.unwrap().unwrap();
    }

    #[tokio::test]
    #[serial]
    async fn loads_and_persists_state() {
        let clock = Arc::new(FakeClock::new());
        let bsec = create_minimal_subscribed_bsec(clock.clone());

        let state = Arc::new(std::sync::RwLock::new(Some(bsec.get_state().unwrap())));
        let persist_state = MockPersistState { state: state.clone() };

        let (monitor, rx) = bsec_monitor(bsec, persist_state, clock.clone());
        let join_handle = tokio::task::spawn(monitor.monitoring_loop());
        *state.write().unwrap() = None;
        rx.initiate_shutdown.send(()).unwrap();
        let (bsec, _) = join_handle.await.unwrap().unwrap();
        assert_eq!(*state.read().unwrap(), Some(bsec.get_state().unwrap()));
    }

    #[tokio::test]
    #[serial]
    async fn autosaves_state() {
        let clock = Arc::new(FakeClock::new());
        let bsec = create_minimal_subscribed_bsec(clock.clone());

        let state = Arc::new(std::sync::RwLock::new(None));
        let persist_state = MockPersistState { state: state.clone() };

        let (monitor, rx) = bsec_monitor(bsec, persist_state, clock.clone());
        let join_handle = tokio::task::spawn(monitor.monitoring_loop());

        for _ in 0..70 {
            clock.sleep(Duration::from_secs(1)).await;
            tokio::task::yield_now().await
        }

        assert!(state.read().unwrap().is_some());
        rx.initiate_shutdown.send(()).unwrap();
        join_handle.await.unwrap().unwrap();
    }
}
