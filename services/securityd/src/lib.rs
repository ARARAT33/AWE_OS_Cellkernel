//! AWEOS User-Space Security Service (`securityd`)
//!
//! Handles TPM 2.0 key storage, PCR measurement verification,
//! capability token issuance/validation, and security audit logging.

#![no_std]

pub mod crypto;
pub use crypto::{
    chacha20poly1305_decrypt, chacha20poly1305_encrypt, ed25519_keypair_from_seed, ed25519_sign,
    ed25519_verify, hkdf_sha256, hmac_sha256, sha256, sha512, x25519_public_from_secret,
    x25519_shared,
};

pub const MAX_PCR_REGISTERS: usize = 24;
pub const MAX_KEYS: usize = 32;
pub const MAX_CAPABILITY_TOKENS: usize = 64;
pub const SHA256_DIGEST_LEN: usize = 32;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct PcrMeasurement {
    pub index: u8,
    pub digest: [u8; SHA256_DIGEST_LEN],
}

#[derive(Debug, Clone, Copy)]
pub struct SecurityToken {
    pub token_id: u64,
    pub subject_pid: u32,
    pub capabilities: u64,
    pub issued_at: u64,
    pub expires_at: u64,
    pub hmac_signature: [u8; 16],
}

impl SecurityToken {
    pub fn is_valid_at(&self, timestamp: u64, required_cap: u64) -> bool {
        if timestamp < self.issued_at || timestamp > self.expires_at {
            return false;
        }
        (self.capabilities & required_cap) == required_cap
    }
}

#[derive(Debug, Clone, Copy)]
pub struct VaultKey {
    pub key_id: u32,
    pub owner_pid: u32,
    pub key_data: [u8; 32],
}

/// Security Daemon Manager Instance.
#[derive(Debug)]
pub struct SecurityDaemon {
    pcr_banks: [[u8; SHA256_DIGEST_LEN]; MAX_PCR_REGISTERS],
    keys: [Option<VaultKey>; MAX_KEYS],
    tokens: [Option<SecurityToken>; MAX_CAPABILITY_TOKENS],
    secret_salt: [u8; 16],
    token_counter: u64,
    key_counter: u32,
}

impl SecurityDaemon {
    pub const fn new(secret_salt: [u8; 16]) -> Self {
        Self {
            pcr_banks: [[0u8; SHA256_DIGEST_LEN]; MAX_PCR_REGISTERS],
            keys: [None; MAX_KEYS],
            tokens: [None; MAX_CAPABILITY_TOKENS],
            secret_salt,
            token_counter: 1,
            key_counter: 1,
        }
    }

    pub fn extend_pcr(
        &mut self,
        pcr_index: usize,
        event_digest: &[u8; SHA256_DIGEST_LEN],
    ) -> Result<(), &'static str> {
        if pcr_index >= MAX_PCR_REGISTERS {
            return Err("PCR index out of bounds");
        }
        for (byte, &evt) in self.pcr_banks[pcr_index]
            .iter_mut()
            .zip(event_digest.iter())
        {
            *byte ^= evt;
        }
        Ok(())
    }

    pub fn get_pcr(&self, pcr_index: usize) -> Result<[u8; SHA256_DIGEST_LEN], &'static str> {
        if pcr_index >= MAX_PCR_REGISTERS {
            return Err("PCR index out of bounds");
        }
        Ok(self.pcr_banks[pcr_index])
    }

    pub fn store_key(&mut self, owner_pid: u32, key_data: [u8; 32]) -> Result<u32, &'static str> {
        let kid = self.key_counter;
        for slot in self.keys.iter_mut() {
            if slot.is_none() {
                *slot = Some(VaultKey {
                    key_id: kid,
                    owner_pid,
                    key_data,
                });
                self.key_counter += 1;
                return Ok(kid);
            }
        }
        Err("Vault key capacity reached")
    }

    pub fn get_key(&self, key_id: u32, requester_pid: u32) -> Result<[u8; 32], &'static str> {
        for key in self.keys.iter().flatten() {
            if key.key_id == key_id {
                if key.owner_pid != requester_pid {
                    return Err("Access denied: key ownership mismatch");
                }
                return Ok(key.key_data);
            }
        }
        Err("Key not found")
    }

    pub fn issue_token(
        &mut self,
        subject_pid: u32,
        capabilities: u64,
        now: u64,
        ttl: u64,
    ) -> Result<SecurityToken, &'static str> {
        let tid = self.token_counter;
        let expires_at = now + ttl;

        let mut payload = [0u8; 32];
        payload[0..8].copy_from_slice(&tid.to_le_bytes());
        payload[8..12].copy_from_slice(&subject_pid.to_le_bytes());
        payload[12..20].copy_from_slice(&capabilities.to_le_bytes());
        payload[20..28].copy_from_slice(&now.to_le_bytes());
        payload[28..32].copy_from_slice(&(ttl as u32).to_le_bytes());

        let full_hmac = hmac_sha256(&self.secret_salt, &payload);
        let mut hmac_sig = [0u8; 16];
        hmac_sig.copy_from_slice(&full_hmac[..16]);

        let token = SecurityToken {
            token_id: tid,
            subject_pid,
            capabilities,
            issued_at: now,
            expires_at,
            hmac_signature: hmac_sig,
        };

        for slot in self.tokens.iter_mut() {
            if slot.is_none() {
                *slot = Some(token);
                self.token_counter += 1;
                return Ok(token);
            }
        }
        Err("Capability token vault full")
    }

    pub fn validate_token(
        &self,
        token_id: u64,
        subject_pid: u32,
        required_cap: u64,
        now: u64,
    ) -> bool {
        for t in self.tokens.iter().flatten() {
            if t.token_id == token_id && t.subject_pid == subject_pid {
                return t.is_valid_at(now, required_cap);
            }
        }
        false
    }
}

impl Default for SecurityDaemon {
    fn default() -> Self {
        Self::new([0u8; 16])
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_securityd_pcr_keys_and_tokens() {
        let mut sec = SecurityDaemon::new([0xA5; 16]);

        let digest = [0x11; SHA256_DIGEST_LEN];
        sec.extend_pcr(0, &digest).unwrap();
        assert_eq!(sec.get_pcr(0).unwrap(), digest);

        let kid = sec.store_key(1001, [0xEE; 32]).unwrap();
        assert_eq!(sec.get_key(kid, 1001).unwrap(), [0xEE; 32]);
        assert!(sec.get_key(kid, 9999).is_err());

        let token = sec.issue_token(1001, 0b101, 1000, 500).unwrap();
        assert!(sec.validate_token(token.token_id, 1001, 0b001, 1200));
        assert!(!sec.validate_token(token.token_id, 1001, 0b010, 1200));
        assert!(!sec.validate_token(token.token_id, 1001, 0b001, 1600));
    }
}
