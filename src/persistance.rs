use super::monitor::PersistState;

#[derive(Default)]
pub struct NoPersistState {}

impl PersistState for NoPersistState {
    type Error = std::convert::Infallible;
    fn load_state(&mut self) -> Result<Option<Vec<u8>>, Self::Error> {
        Ok(None)
    }
    fn save_state(&mut self, _: &[u8]) -> Result<(), Self::Error> {
        Ok(())
    }
}
