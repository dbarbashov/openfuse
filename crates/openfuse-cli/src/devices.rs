#[cfg(feature = "hid")]
pub fn enumerate(json: bool) -> Result<(), String> {
    use openfuse_core::catalog::{FENDER_VENDOR_ID, find_device};

    let api = hidapi::HidApi::new().map_err(|error| format!("HID enumeration failed: {error}"))?;
    let devices: Vec<_> = api
        .device_list()
        .filter(|device| device.vendor_id() == FENDER_VENDOR_ID)
        .map(|device| {
            serde_json::json!({
                "id": openfuse_core::hex::encode(device.path().to_bytes()),
                "serial": device.serial_number(),
                "vendor_id": device.vendor_id(),
                "product_id": device.product_id(),
                "manufacturer": device.manufacturer_string(),
                "product": device.product_string(),
                "interface_number": device.interface_number(),
                "usage_page": device.usage_page(),
                "usage": device.usage(),
                "catalog_identity": find_device(device.vendor_id(), device.product_id()),
            })
        })
        .collect();
    if json {
        println!(
            "{}",
            serde_json::to_string_pretty(&devices).map_err(|error| error.to_string())?
        );
    } else {
        println!(
            "{} Fender HID interfaces found; no device opened or report sent.",
            devices.len()
        );
        for device in devices {
            let pid = device["product_id"].as_u64().unwrap_or_default();
            let name = device["catalog_identity"]["name"]
                .as_str()
                .unwrap_or("unknown product ID");
            println!(
                "1ed8:{pid:04x}  {name} (interface {})",
                device["interface_number"]
            );
        }
        println!(
            "Enumeration covers HID interfaces only; absence does not prove no amplifier is connected."
        );
    }
    Ok(())
}

#[cfg(not(feature = "hid"))]
pub fn enumerate(_json: bool) -> Result<(), String> {
    Err("HID enumeration is not compiled in; run cargo run -p openfuse-cli --features hid -- devices".into())
}
