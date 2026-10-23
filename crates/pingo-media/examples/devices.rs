fn main() -> pingo_media::Result<()> {
    for device in pingo_media::audio::device::list_devices()? {
        println!(
            "{} {}{}",
            if device.input { "Input " } else { "Output" },
            device.name,
            if device.is_default { " (default)" } else { "" }
        );
    }
    Ok(())
}
