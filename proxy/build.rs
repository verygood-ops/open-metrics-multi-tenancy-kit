extern crate protoc_rust;

fn main() {
    protoc_rust::Codegen::new()
        .out_dir("src/proto")
        .inputs(&["config/prometheus.proto"])
        .run()
        .expect("protoc");
}
