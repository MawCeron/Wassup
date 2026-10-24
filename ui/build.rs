fn main() -> std::io::Result<()> {
    prost_build::Config::new()
        // Generated oneofs hold whole messages; boxing them is not worth it here.
        .enum_attribute(".", "#[allow(clippy::large_enum_variant)]")
        .compile_protos(&["../proto/ipc.proto"], &["../proto"])
}
