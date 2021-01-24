use super::bsec::{self, BmeSensor, Bsec, OutputSignal, Time};
use anyhow::Result;
use nb::block;
use tokio::sync::{oneshot, watch};
use tokio::task::JoinHandle;
use tokio::time::{sleep, Duration};

pub struct Monitor {
    pub current: watch::Receiver<Vec<OutputSignal>>,
    request_shutdown: oneshot::Sender<()>,
    join_handle: JoinHandle<Result<()>>,
}

impl Monitor {
    pub async fn start<'t, S, T>(bsec: Bsec<'static, S, T>, time: &'static T) -> Result<Self>
    where
        S: BmeSensor + 'static,
        T: Time,
        S::Error: std::error::Error + Send + Sync + 'static,
    {
        let mut bsec = bsec;
        let (set_current, current) = watch::channel(Self::next_measurement(&mut bsec, time).await?);
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
        bsec: Bsec<'static, S, T>,
        time: &'static T,
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
            set_current.send(Self::next_measurement(&mut bsec, time).await?)?;
        }
        Ok(())
    }

    async fn next_measurement<'t, S: BmeSensor, T: Time>(
        bsec: &mut Bsec<'t, S, T>,
        time: &'static T,
    ) -> Result<Vec<OutputSignal>, bsec::Error<S::Error>> {
        let sleep_duration = bsec.next_measurement() - time.timestamp_ns();
        if sleep_duration > 0 {
            sleep(Duration::from_nanos(sleep_duration as u64)).await;
        }
        let duration = block!(bsec.start_next_measurement())?;
        sleep(duration).await;
        block!(bsec.process_last_measurement())
    }
}
