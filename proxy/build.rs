extern crate protoc_rust;

use std::fs::{canonicalize,copy};
use std::path::PathBuf;
use std::process::exit;



fn main() {
    match copy(canonicalize(PathBuf::from("../config/prometheus.proto")).unwrap(), "prometheus.proto") {
        Err(_) => {
            println!("Failed to compile prometheus protobuf!");
            exit(7);
        },
        Ok(_i) => {
            protoc_rust::Codegen::new()
                .out_dir("src/proto")
                .inputs(&["prometheus.proto"])
                .run()
                .expect("protoc");
        }
    };

}
