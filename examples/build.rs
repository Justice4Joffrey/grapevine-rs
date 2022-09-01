static OUT_DIR: &str = "src/generated";

fn main() -> Result<(), Box<dyn std::error::Error>> {
    // by default, Prost will generate the output in the build, which is less
    // helpful in development as your IDE won't be able to infer any of the
    // types. Instead, we generate a file in to the source.
    //
    // Note: to use this file, you'll have to use `include!` instead of
    // `include_proto!`.
    if std::fs::metadata(OUT_DIR).is_ok() {
        std::fs::remove_dir_all(OUT_DIR)?;
    }
    std::fs::create_dir(OUT_DIR)?;

    tonic_build::configure()
        .build_client(true)
        .build_server(true)
        .out_dir(OUT_DIR)
        .compile(&["proto/order_book.proto"], &["proto"])?;
    Ok(())
}
