use {
    std::{env, io},
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
    Ok(())
}
