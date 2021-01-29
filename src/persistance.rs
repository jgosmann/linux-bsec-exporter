use super::monitor::PersistState;
use std::fs::File;
use std::io::{Read, Write};
use std::path::Path;

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

pub struct StateFile<P: AsRef<Path>> {
    path: P,
}

impl<P: AsRef<Path>> StateFile<P> {
    pub fn new(path: P) -> Self {
        Self { path }
    }
}

impl<P: AsRef<Path>> PersistState for StateFile<P> {
    type Error = std::io::Error;

    fn load_state(&mut self) -> Result<Option<Vec<u8>>, Self::Error> {
        match File::open(self.path.as_ref()) {
            Ok(mut file) => {
                let mut state = vec![];
                file.read_to_end(&mut state)?;
                Ok(Some(state))
            }
            Err(error) => match error.kind() {
                std::io::ErrorKind::NotFound => Ok(None),
                _ => Err(error),
            },
        }
    }

    fn save_state(&mut self, state: &[u8]) -> Result<(), Self::Error> {
        let mut file = File::create(self.path.as_ref())?;
        file.write_all(state)
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::env::temp_dir;

    #[test]
    fn test_state_file_roundtrips() {
        let path = temp_dir().join("state_file");

        let mut state_file = StateFile::new(path);
        assert_eq!(state_file.load_state().unwrap(), None);

        let initial_state = vec![1u8, 2, 3, 4];
        assert_eq!(state_file.save_state(&initial_state).unwrap(), ());
        assert_eq!(state_file.load_state().unwrap(), Some(initial_state));

        let overwritten_state = vec![5u8, 6, 7, 8];
        assert_eq!(state_file.save_state(&overwritten_state).unwrap(), ());
        assert_eq!(state_file.load_state().unwrap(), Some(overwritten_state));
    }
}
