fn main() -> Result<(), Box<dyn std::error::Error>> {
    // let includes: &[&str; 0] = &[];
    // tonic_build::configure().out_dir(".").compile(&["api/proto/admin/v1/karma.proto"], includes)?;
    tonic_build::configure().compile(&["idl/api/proto/admin/v1/karma.proto", "idl/api/proto/admin/v1/plugins.proto"], &["idl/api/proto"])?;

    Ok(())
}
