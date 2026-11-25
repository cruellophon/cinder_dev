use crate::PersistedState;
use sha2::{Digest, Sha256};
use std::path::Path;
use uuid::Uuid;

const DPAPI_PREFIX: &str = "dpapi:v1:";
const SEALED_PREFIX: &str = "sealed:v1:";
#[cfg(windows)]
const DPAPI_ENTROPY: &[u8] = b"cinder-state-vault-v1";

fn for_each_secret_mut(state: &mut PersistedState, mut f: impl FnMut(&mut String)) {
    f(&mut state.settings.api_keys.gemini_api_key);
    f(&mut state.settings.api_keys.anthropic_api_key);
    f(&mut state.settings.api_keys.openrouter_api_key);
    f(&mut state.settings.api_keys.openai_compat_api_key);
    f(&mut state.settings.api_keys.huggingface_token);
    f(&mut state.settings.api_keys.brave_search_api_key);
    for srv in &mut state.mcp_servers {
        for val in srv.env.values_mut() {
            f(val);
        }
    }
}

fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    let mut k_block = [0u8; 64];
    if key.len() > 64 {
        let mut h = Sha256::new();
        h.update(key);
        k_block[..32].copy_from_slice(&h.finalize());
    } else {
        k_block[..key.len()].copy_from_slice(key);
    }

    let mut ipad = [0x36u8; 64];
    let mut opad = [0x5cu8; 64];
    for i in 0..64 {
        ipad[i] ^= k_block[i];
        opad[i] ^= k_block[i];
    }

    let mut inner = Sha256::new();
    inner.update(ipad);
    inner.update(data);
    let inner_digest = inner.finalize();

    let mut outer = Sha256::new();
    outer.update(opad);
    outer.update(inner_digest);
    let out: [u8; 32] = outer.finalize().into();
    out
}

fn constant_time_eq(a: &[u8], b: &[u8]) -> bool {
    if a.len() != b.len() {
        return false;
    }
    let mut diff = 0u8;
    for (&x, &y) in a.iter().zip(b.iter()) {
        diff |= x ^ y;
    }
    diff == 0
}

fn derive_subkey(domain: &[u8], master_key: &[u8; 32]) -> [u8; 32] {
    let mut h = Sha256::new();
    h.update(domain);
    h.update(master_key);
    h.finalize().into()
}

fn keystream_xor(enc_key: &[u8; 32], nonce: &[u8; 16], input: &[u8]) -> Vec<u8> {
    let mut out = Vec::with_capacity(input.len());
    for (block_idx, chunk) in input.chunks(32).enumerate() {
        let mut h = Sha256::new();
        h.update(enc_key);
        h.update(nonce);
        h.update((block_idx as u64).to_be_bytes());
        let block = h.finalize();
        for (i, &b) in chunk.iter().enumerate() {
            out.push(b ^ block[i]);
        }
    }
    out
}

#[cfg_attr(windows, allow(dead_code))]
pub(crate) fn seal_with_key(plaintext: &str, master_key: &[u8; 32]) -> String {
    let nonce: [u8; 16] = *Uuid::new_v4().as_bytes();
    let enc_key = derive_subkey(b"cinder-vault-enc-v1", master_key);
    let mac_key = derive_subkey(b"cinder-vault-mac-v1", master_key);
    let ciphertext = keystream_xor(&enc_key, &nonce, plaintext.as_bytes());

    let mut mac_input = Vec::with_capacity(SEALED_PREFIX.len() + nonce.len() + ciphertext.len());
    mac_input.extend_from_slice(SEALED_PREFIX.as_bytes());
    mac_input.extend_from_slice(&nonce);
    mac_input.extend_from_slice(&ciphertext);
    let tag = hmac_sha256(&mac_key, &mac_input);

    format!(
        "{SEALED_PREFIX}{}:{}:{}",
        hex::encode(nonce),
        hex::encode(&ciphertext),
        hex::encode(tag)
    )
}

pub(crate) fn unseal_with_key(encoded: &str, master_key: &[u8; 32]) -> anyhow::Result<String> {
    let body = encoded
        .strip_prefix(SEALED_PREFIX)
        .ok_or_else(|| anyhow::anyhow!("Missing sealed:v1 prefix"))?;
    let mut parts = body.splitn(3, ':');
    let nonce_hex = parts
        .next()
        .ok_or_else(|| anyhow::anyhow!("Missing nonce"))?;
    let ct_hex = parts
        .next()
        .ok_or_else(|| anyhow::anyhow!("Missing ciphertext"))?;
    let tag_hex = parts.next().ok_or_else(|| anyhow::anyhow!("Missing tag"))?;

    let nonce_bytes = hex::decode(nonce_hex)?;
    if nonce_bytes.len() != 16 {
        anyhow::bail!("Invalid sealed nonce length");
    }
    let mut nonce = [0u8; 16];
    nonce.copy_from_slice(&nonce_bytes);

    let ciphertext = hex::decode(ct_hex)?;
    let tag_bytes = hex::decode(tag_hex)?;
    if tag_bytes.len() != 32 {
        anyhow::bail!("Invalid sealed tag length");
    }

    let mac_key = derive_subkey(b"cinder-vault-mac-v1", master_key);
    let mut mac_input = Vec::with_capacity(SEALED_PREFIX.len() + nonce.len() + ciphertext.len());
    mac_input.extend_from_slice(SEALED_PREFIX.as_bytes());
    mac_input.extend_from_slice(&nonce);
    mac_input.extend_from_slice(&ciphertext);
    let expected_tag = hmac_sha256(&mac_key, &mac_input);

    if !constant_time_eq(&expected_tag, &tag_bytes) {
        anyhow::bail!("Sealed secret authentication failed");
    }

    let enc_key = derive_subkey(b"cinder-vault-enc-v1", master_key);
    let plaintext = keystream_xor(&enc_key, &nonce, &ciphertext);
    Ok(String::from_utf8(plaintext)?)
}

#[cfg(windows)]
mod win_dpapi {
    use super::{DPAPI_ENTROPY, DPAPI_PREFIX};

    const CRYPTPROTECT_UI_FORBIDDEN: u32 = 0x01;

    #[repr(C)]
    struct DataBlob {
        cb_data: u32,
        pb_data: *mut u8,
    }

    #[link(name = "crypt32")]
    extern "system" {
        fn CryptProtectData(
            p_data_in: *const DataBlob,
            sz_data_descr: *const u16,
            p_optional_entropy: *const DataBlob,
            pv_reserved: *mut core::ffi::c_void,
            p_prompt_struct: *mut core::ffi::c_void,
            dw_flags: u32,
            p_data_out: *mut DataBlob,
        ) -> i32;

        fn CryptUnprotectData(
            p_data_in: *const DataBlob,
            ppsz_data_descr: *mut *mut u16,
            p_optional_entropy: *const DataBlob,
            pv_reserved: *mut core::ffi::c_void,
            p_prompt_struct: *mut core::ffi::c_void,
            dw_flags: u32,
            p_data_out: *mut DataBlob,
        ) -> i32;
    }

    #[link(name = "kernel32")]
    extern "system" {
        fn LocalFree(h_mem: *mut core::ffi::c_void) -> *mut core::ffi::c_void;
    }

    pub fn dpapi_protect(plaintext: &str) -> anyhow::Result<String> {
        if plaintext.is_empty() {
            return Ok(String::new());
        }
        let mut raw = plaintext.as_bytes().to_vec();
        let data_in = DataBlob {
            cb_data: u32::try_from(raw.len())
                .map_err(|_| anyhow::anyhow!("Secret too large for DPAPI"))?,
            pb_data: raw.as_mut_ptr(),
        };
        let mut entropy_buf = DPAPI_ENTROPY.to_vec();
        let entropy = DataBlob {
            cb_data: entropy_buf.len() as u32,
            pb_data: entropy_buf.as_mut_ptr(),
        };
        let mut data_out = DataBlob {
            cb_data: 0,
            pb_data: std::ptr::null_mut(),
        };

        let ok = unsafe {
            CryptProtectData(
                &data_in,
                std::ptr::null(),
                &entropy,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut data_out,
            )
        };
        if ok == 0 || data_out.pb_data.is_null() {
            if !data_out.pb_data.is_null() {
                unsafe { LocalFree(data_out.pb_data as *mut _) };
            }
            anyhow::bail!("CryptProtectData failed: {}", std::io::Error::last_os_error());
        }

        let protected = unsafe {
            std::slice::from_raw_parts(data_out.pb_data, data_out.cb_data as usize).to_vec()
        };
        unsafe { LocalFree(data_out.pb_data as *mut _) };
        Ok(format!("{DPAPI_PREFIX}{}", hex::encode(protected)))
    }

    pub fn dpapi_unprotect(encoded: &str) -> anyhow::Result<String> {
        let hex_body = encoded
            .strip_prefix(DPAPI_PREFIX)
            .ok_or_else(|| anyhow::anyhow!("Missing dpapi:v1 prefix"))?;
        let mut raw = hex::decode(hex_body)?;
        if raw.is_empty() {
            return Ok(String::new());
        }
        let data_in = DataBlob {
            cb_data: u32::try_from(raw.len())
                .map_err(|_| anyhow::anyhow!("DPAPI blob too large"))?,
            pb_data: raw.as_mut_ptr(),
        };
        let mut entropy_buf = DPAPI_ENTROPY.to_vec();
        let entropy = DataBlob {
            cb_data: entropy_buf.len() as u32,
            pb_data: entropy_buf.as_mut_ptr(),
        };
        let mut data_out = DataBlob {
            cb_data: 0,
            pb_data: std::ptr::null_mut(),
        };

        let ok = unsafe {
            CryptUnprotectData(
                &data_in,
                std::ptr::null_mut(),
                &entropy,
                std::ptr::null_mut(),
                std::ptr::null_mut(),
                CRYPTPROTECT_UI_FORBIDDEN,
                &mut data_out,
            )
        };
        if ok == 0 || data_out.pb_data.is_null() {
            if !data_out.pb_data.is_null() {
                unsafe { LocalFree(data_out.pb_data as *mut _) };
            }
            anyhow::bail!(
                "CryptUnprotectData failed: {}",
                std::io::Error::last_os_error()
            );
        }

        let plain_bytes = unsafe {
            std::slice::from_raw_parts(data_out.pb_data, data_out.cb_data as usize).to_vec()
        };
        unsafe { LocalFree(data_out.pb_data as *mut _) };
        Ok(String::from_utf8(plain_bytes)?)
    }
}

fn vault_key_path(state_file: &Path) -> std::path::PathBuf {
    state_file
        .parent()
        .unwrap_or_else(|| Path::new("."))
        .join("vault.key")
}

fn try_load_vault_key(state_file: &Path) -> Option<[u8; 32]> {
    let raw = std::fs::read_to_string(vault_key_path(state_file)).ok()?;
    let bytes = hex::decode(raw.trim()).ok()?;
    if bytes.len() != 32 {
        return None;
    }
    let mut key = [0u8; 32];
    key.copy_from_slice(&bytes);
    Some(key)
}

#[cfg(not(windows))]
fn load_or_create_vault_key(state_file: &Path) -> anyhow::Result<[u8; 32]> {
    if let Some(existing) = try_load_vault_key(state_file) {
        return Ok(existing);
    }
    let kpath = vault_key_path(state_file);
    if let Some(parent) = kpath.parent() {
        std::fs::create_dir_all(parent)?;
    }
    let mut master = [0u8; 32];
    master[..16].copy_from_slice(Uuid::new_v4().as_bytes());
    master[16..].copy_from_slice(Uuid::new_v4().as_bytes());

    use std::io::Write;
    use std::os::unix::fs::OpenOptionsExt;
    let mut f = std::fs::OpenOptions::new()
        .create(true)
        .write(true)
        .truncate(true)
        .mode(0o600)
        .open(&kpath)?;
    f.write_all(hex::encode(master).as_bytes())?;
    Ok(master)
}

pub fn seal_state_for_disk(
    state: &PersistedState,
    state_file: &Path,
) -> anyhow::Result<PersistedState> {
    let mut out = state.clone();
    #[cfg(windows)]
    let _ = state_file;
    #[cfg(not(windows))]
    let master_key = load_or_create_vault_key(state_file)?;

    let mut err: Option<anyhow::Error> = None;
    for_each_secret_mut(&mut out, |val| {
        if err.is_some() || val.trim().is_empty() {
            return;
        }
        #[cfg(windows)]
        {
            if !val.starts_with(DPAPI_PREFIX) {
                match win_dpapi::dpapi_protect(val) {
                    Ok(sealed) => *val = sealed,
                    Err(e) => err = Some(e),
                }
            }
        }
        #[cfg(not(windows))]
        {
            if !val.starts_with(SEALED_PREFIX) {
                *val = seal_with_key(val, &master_key);
            }
        }
    });
    if let Some(e) = err {
        return Err(e);
    }
    Ok(out)
}

pub fn unseal_state_in_place(state: &mut PersistedState, state_file: &Path) {
    let sealed_key = try_load_vault_key(state_file);
    for_each_secret_mut(state, |val| {
        if val.starts_with(DPAPI_PREFIX) {
            #[cfg(windows)]
            {
                *val = win_dpapi::dpapi_unprotect(val).unwrap_or_default();
            }
            #[cfg(not(windows))]
            {
                val.clear();
            }
        } else if val.starts_with(SEALED_PREFIX) {
            *val = sealed_key
                .as_ref()
                .and_then(|k| unseal_with_key(val, k).ok())
                .unwrap_or_default();
        }
    });
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn sealed_v1_roundtrip_and_tamper_rejection() {
        let key = [0x42u8; 32];
        let other_key = [0x99u8; 32];
        let secret = "AIzaSy-secret-api-key-123456";

        let c1 = seal_with_key(secret, &key);
        let c2 = seal_with_key(secret, &key);
        assert!(c1.starts_with(SEALED_PREFIX));
        assert_ne!(c1, c2, "distinct nonces must yield distinct ciphertexts");
        assert_eq!(unseal_with_key(&c1, &key).unwrap(), secret);
        assert_eq!(unseal_with_key(&c2, &key).unwrap(), secret);
        assert!(unseal_with_key(&c1, &other_key).is_err());

        // Tamper one hex character in ciphertext/MAC
        let mut tampered = c1.clone();
        let last = tampered.pop().unwrap();
        tampered.push(if last == '0' { '1' } else { '0' });
        assert!(unseal_with_key(&tampered, &key).is_err());
    }
}
