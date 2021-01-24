use super::bsec::{self, BmeSensor, Bsec, OutputSignal, Time};
use anyhow::Result;
use nb::block;
use std::sync::Arc;
use tokio::sync::{oneshot, watch};
use tokio::task::JoinHandle;
use tokio::time::{sleep, Duration};

pub struct Monitor {
    pub current: watch::Receiver<Vec<OutputSignal>>,
    request_shutdown: oneshot::Sender<()>,
    join_handle: JoinHandle<Result<()>>,
}

impl Monitor {
    pub async fn start<S, T>(bsec: Bsec<S, T, Arc<T>>, time: Arc<T>) -> Result<Self>
    where
        S: BmeSensor + 'static,
        T: Time + 'static,
        S::Error: std::error::Error + Send + Sync + 'static,
    {
        let mut bsec = bsec;
        let (set_current, current) =
            watch::channel(Self::next_measurement(&mut bsec, time.clone()).await?);
        let (request_shutdown, shutdown_requested) = oneshot::channel();
        let join_handle = tokio::task::spawn_local(Self::monitoring_loop(
            bsec,
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

    async fn monitoring_loop<'t, S, T>(
        bsec: Bsec<S, T, Arc<T>>,
        time: Arc<T>,
        set_current: watch::Sender<Vec<OutputSignal>>,
        shutdown_requested: oneshot::Receiver<()>,
    ) -> Result<()>
    where
        S: BmeSensor + 'static,
        T: Time,
        S::Error: std::error::Error + Send + Sync + 'static,
    {
        let mut bsec = bsec;
        let mut shutdown_requested = shutdown_requested;
        while shutdown_requested.try_recv().is_err() {
            set_current.send(Self::next_measurement(&mut bsec, time.clone()).await?)?;
        }
        Ok(())
    }

    async fn next_measurement<S: BmeSensor, T: Time>(
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

    async fn stop(self) -> Result<(), anyhow::Error> {
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
                let mut monitor = Monitor::start(bsec, time.clone()).await.unwrap();
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
}
