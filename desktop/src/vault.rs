use anyhow::{Context, Result};
use zeroize::Zeroizing;
#[cfg(windows)]
pub fn protect(input: &[u8], binding: &str, encrypt: bool) -> Result<Zeroizing<Vec<u8>>> {
    use std::ptr::{null, null_mut};
    use windows_sys::Win32::{
        Foundation::LocalFree,
        Security::Cryptography::{
            CRYPT_INTEGER_BLOB, CRYPTPROTECT_UI_FORBIDDEN, CryptProtectData, CryptUnprotectData,
        },
    };
    use zeroize::Zeroize;
    fn blob(v: &[u8]) -> Result<CRYPT_INTEGER_BLOB> {
        Ok(CRYPT_INTEGER_BLOB {
            cbData: v.len().try_into()?,
            pbData: v.as_ptr().cast_mut(),
        })
    }
    let input = blob(input)?;
    let entropy = blob(binding.as_bytes())?;
    let mut out = CRYPT_INTEGER_BLOB {
        cbData: 0,
        pbData: null_mut(),
    };
    // DPAPI 分配返回缓冲区。复制结果后擦除明文，并用同一系统分配器释放。
    unsafe {
        let ok = if encrypt {
            CryptProtectData(
                &input,
                null(),
                &entropy,
                null(),
                null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            )
        } else {
            CryptUnprotectData(
                &input,
                null_mut(),
                &entropy,
                null(),
                null(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut out,
            )
        };
        if ok == 0 {
            return Err(std::io::Error::last_os_error())
                .context("Windows 无法保护或解锁此节点的凭据");
        }
        anyhow::ensure!(!out.pbData.is_null(), "Windows 凭据为空");
        let buffer = std::slice::from_raw_parts_mut(out.pbData, out.cbData as usize);
        let bytes = Zeroizing::new(buffer.to_vec());
        buffer.zeroize();
        LocalFree(out.pbData.cast());
        Ok(bytes)
    }
}
#[cfg(not(windows))]
pub fn protect(_: &[u8], _: &str, _: bool) -> Result<Zeroizing<Vec<u8>>> {
    None.context("保存凭据需要 Windows 用户保护服务")
}
#[cfg(all(test, windows))]
mod tests {
    #[test]
    fn encrypted_secret_is_bound_to_the_node() {
        let text = format!("{:032x}", rand::random::<u128>());
        let sealed = super::protect(text.as_bytes(), "node-a", true).unwrap();
        assert_ne!(sealed.as_slice(), text.as_bytes());
        assert_eq!(
            super::protect(&sealed, "node-a", false).unwrap().as_slice(),
            text.as_bytes()
        );
        assert!(super::protect(&sealed, "node-b", false).is_err());
    }
}
