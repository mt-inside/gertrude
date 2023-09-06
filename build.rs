fn main() -> Result<(), Box<dyn std::error::Error>> {
    // let includes: &[&str; 0] = &[];
    // tonic_build::configure().out_dir(".").compile(&["api/proto/admin/v1/karma.proto"], includes)?;
    tonic_build::configure().compile(&["api/proto/admin/v1/karma.proto", "api/proto/admin/v1/plugins.proto"], &["api/proto"])?;

    Ok(())
}
