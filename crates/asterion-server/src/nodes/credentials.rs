use super::error::ApiError;
const HEX_DIGITS: &[u8; 16] = b"0123456789abcdef";
pub(super) fn random_hex<const N: usize>() -> Result<String, ApiError> {
    let mut bytes = [0; N];
    getrandom::fill(&mut bytes).map_err(|_| ApiError::internal("os_random"))?;
    let mut encoded = String::with_capacity(N * 2);
    for byte in bytes {
        encoded.push(char::from(HEX_DIGITS[usize::from(byte >> 4)]));
        encoded.push(char::from(HEX_DIGITS[usize::from(byte & 0xf)]));
    }
    Ok(encoded)
}
