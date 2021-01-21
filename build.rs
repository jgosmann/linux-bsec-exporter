extern crate bindgen;

use std::env;
use std::path::PathBuf;

fn main() {
    let out_path = PathBuf::from(env::var("OUT_DIR").unwrap());

    println!("cargo:rerun-if-changed=bsec/bsec_interface.h");
    println!("cargo:rerun-if-changed=bsec/bsec_datatypes.h");
    println!("cargo:rustc-link-lib=static=algobsec");
    println!("cargo:rustc-link-search=native=bsec");
    let bindings = bindgen::Builder::default()
        .header("bsec/bsec_interface.h")
        .parse_callbacks(Box::new(bindgen::CargoCallbacks))
        .generate()
        .expect("Unable to generate BSEC bindings.");
    bindings
        .write_to_file(out_path.join("bsec_bindings.rs"))
        .expect("Could not write BSEC bindings.");
}
