use crate::error::IpaError;

pub fn generate_guid() -> Result<String, IpaError> {
    let mac = mac_address::get_mac_address()
        .map_err(|e| IpaError::Other(format!("failed to get MAC address: {e}")))?
        .ok_or(IpaError::NoGuid)?;

    let guid = mac
        .bytes()
        .iter()
        .map(|b| format!("{b:02X}"))
        .collect::<String>();

    Ok(guid)
}
