use {
    std::{env, fs, io},
    winres::WindowsResource,
};

fn main() -> io::Result<()> {
    if env::var_os("CARGO_CFG_WINDOWS").is_some() {
        WindowsResource::new()
            // This path can be absolute, or relative to your crate root.
            .set_icon("assets/application.ico")
            .set_version_info(winres::VersionInfo::PRODUCTVERSION, 0x0001_0000_0000_0000)
            .compile()?;
    }
    let mut out_dir = env::var("OUT_DIR").unwrap();
    out_dir.push_str("../../../../tracer.ino");
    let _ = fs::copy("./assets/tracer/tracer.ino", out_dir);
    Ok(())
}
