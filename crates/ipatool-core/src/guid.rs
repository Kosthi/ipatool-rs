use crate::error::IpaError;

/// The GUID sent to Apple and the hardware ID the SAP handshake binds its
/// session key to are two encodings of the same MAC address bytes. Deriving
/// them separately would let them drift apart, which Apple rejects, so they are
/// produced together and carried together.
#[derive(Debug, Clone)]
pub struct MachineIdentity {
    pub guid: String,
    pub hardware_id: Vec<u8>,
}

pub fn generate_machine_identity() -> Result<MachineIdentity, IpaError> {
    let mac = mac_address::get_mac_address()
        .map_err(|e| IpaError::Other(format!("failed to get MAC address: {e}")))?
        .ok_or(IpaError::NoGuid)?;

    let hardware_id = mac.bytes().to_vec();

    // Apple's SAP setup call rejects anything outside this range.
    if hardware_id.is_empty() || hardware_id.len() > 20 {
        return Err(IpaError::Other(format!(
            "hardware address must be 1-20 bytes, got {}",
            hardware_id.len()
        )));
    }

    let guid = hardware_id.iter().map(|b| format!("{b:02X}")).collect();

    Ok(MachineIdentity { guid, hardware_id })
}

pub fn generate_guid() -> Result<String, IpaError> {
    Ok(generate_machine_identity()?.guid)
}
