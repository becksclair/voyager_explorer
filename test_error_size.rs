use std::mem::size_of;
use voyager_explorer::error::{ConfigError, VoyagerError};

fn main() {
    println!("ConfigError size: {} bytes", size_of::<ConfigError>());
    println!("VoyagerError size: {} bytes", size_of::<VoyagerError>());
    println!("PathBuf size: {} bytes", size_of::<std::path::PathBuf>());
}
