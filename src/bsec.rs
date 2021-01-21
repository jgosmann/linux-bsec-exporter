pub mod ffi {
    #![allow(non_camel_case_types)]
    #![allow(non_upper_case_globals)]

    include!(concat!(env!("OUT_DIR"), "/bsec_bindings.rs"));
}

pub struct Bsec {
    _disallow_creation_by_member_initialization: (),
}

impl Bsec {
    fn init() -> Result<Self, Error> {
        Ok(Self {
            _disallow_creation_by_member_initialization: (),
        })
    }
}

#[derive(Debug)]
pub enum Error {
    BsecAlreadyInUse,
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
