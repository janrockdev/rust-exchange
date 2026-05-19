use std::error::Error;
use std::{env, path::PathBuf};

fn main() -> Result<(), Box<dyn Error>> {
    let protoc = protoc_bin_vendored::protoc_bin_path()?;
    env::set_var("PROTOC", protoc);

    let out_dir = PathBuf::from(env::var("OUT_DIR").unwrap());
    
    tonic_build::configure()
        .file_descriptor_set_path(out_dir.join("orderbook_descriptor.bin"))
        .compile_protos(&["proto/orderbook.proto"], &["proto"])?;

    Ok(())
}