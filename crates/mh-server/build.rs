fn main() -> std::io::Result<()> {
    #[cfg(windows)]
    {
        let icon = "../../assets/icon.ico";
        println!("cargo:rerun-if-changed={icon}");
        winresource::WindowsResource::new()
            .set_icon(icon)
            .compile()?;
    }
    Ok(())
}
