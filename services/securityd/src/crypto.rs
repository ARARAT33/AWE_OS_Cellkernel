//! Cryptographic utilities for AWE_OS (`awe-securityd`).

pub use ed25519_dalek::{Signature as Ed25519Signature, SigningKey, VerifyingKey};
pub use hmac::{Hmac, KeyInit, Mac};
pub use sha2::{Digest, Sha256, Sha512};

pub type HmacSha256 = Hmac<Sha256>;

pub fn sha256(data: &[u8]) -> [u8; 32] {
    let mut hasher = Sha256::new();
    hasher.update(data);
    hasher.finalize().into()
}

pub fn sha512(data: &[u8]) -> [u8; 64] {
    let mut hasher = Sha512::new();
    hasher.update(data);
    hasher.finalize().into()
}

pub fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    let mut mac = HmacSha256::new_from_slice(key).expect("HMAC can take key of any size");
    mac.update(data);
    mac.finalize().into_bytes().into()
}

pub fn ed25519_keypair_from_seed(seed: &[u8; 32]) -> ([u8; 32], [u8; 64]) {
    let signing_key = SigningKey::from_bytes(seed);
    let verifying_key = signing_key.verifying_key();

    let mut secret_key_bytes = [0u8; 64];
    secret_key_bytes[..32].copy_from_slice(signing_key.as_bytes());
    secret_key_bytes[32..].copy_from_slice(verifying_key.as_bytes());

    (*verifying_key.as_bytes(), secret_key_bytes)
}

pub fn ed25519_sign(secret_key: &[u8; 64], message: &[u8]) -> [u8; 64] {
    let seed: &[u8; 32] = secret_key[..32].try_into().expect("32 bytes seed");
    let signing_key = SigningKey::from_bytes(seed);
    use ed25519_dalek::Signer;
    let signature = signing_key.sign(message);
    signature.to_bytes()
}

pub fn ed25519_verify(public_key: &[u8; 32], message: &[u8], signature: &[u8; 64]) -> bool {
    let Ok(verifying_key) = VerifyingKey::from_bytes(public_key) else {
        return false;
    };
    let sig = ed25519_dalek::Signature::from_bytes(signature);
    verifying_key.verify_strict(message, &sig).is_ok()
}

/// Derive the X25519 public key for a 32-byte static secret.
///
/// The input is an unclamped scalar; clamping is applied by the X25519
/// function itself, exactly as specified in RFC 7748.
pub fn x25519_public_from_secret(secret: &[u8; 32]) -> [u8; 32] {
    let static_secret = x25519_dalek::StaticSecret::from(*secret);
    x25519_dalek::PublicKey::from(&static_secret).to_bytes()
}

/// Complete an X25519 Diffie-Hellman exchange.
///
/// Returns `None` when the peer public key is a low-order point (the shared
/// secret would be all zero and contributes no security), which callers must
/// treat as a handshake failure.
pub fn x25519_shared(secret: &[u8; 32], peer_public: &[u8; 32]) -> Option<[u8; 32]> {
    let static_secret = x25519_dalek::StaticSecret::from(*secret);
    let peer = x25519_dalek::PublicKey::from(*peer_public);
    let shared = static_secret.diffie_hellman(&peer);
    if !shared.was_contributory() {
        return None;
    }
    Some(*shared.as_bytes())
}

/// HKDF-SHA256 expand step (RFC 5869) with an explicit salt.
///
/// Writes exactly `out.len()` bytes of key material. Returns `false` when the
/// requested output length exceeds the RFC 5869 limit (255 * 32 bytes).
pub fn hkdf_sha256(salt: &[u8], ikm: &[u8], info: &[u8], out: &mut [u8]) -> bool {
    let hkdf = hkdf::Hkdf::<Sha256>::new(Some(salt), ikm);
    hkdf.expand(info, out).is_ok()
}

/// ChaCha20-Poly1305 authenticated encryption in place (RFC 8439).
///
/// `buffer` holds the plaintext and is overwritten with ciphertext. Returns
/// the 16-byte Poly1305 tag. `aad` is authenticated but not encrypted.
pub fn chacha20poly1305_encrypt(
    key: &[u8; 32],
    nonce: &[u8; 12],
    aad: &[u8],
    buffer: &mut [u8],
) -> Option<[u8; 16]> {
    use chacha20poly1305::aead::{AeadInOut, KeyInit};

    let cipher = chacha20poly1305::ChaCha20Poly1305::new(key.into());
    let tag = cipher
        .encrypt_inout_detached(nonce.into(), aad, buffer.into())
        .ok()?;
    Some(tag.into())
}

/// ChaCha20-Poly1305 authenticated decryption in place (RFC 8439).
///
/// `buffer` holds the ciphertext and is overwritten with plaintext only when
/// the tag verifies. Returns `false` on authentication failure.
pub fn chacha20poly1305_decrypt(
    key: &[u8; 32],
    nonce: &[u8; 12],
    aad: &[u8],
    buffer: &mut [u8],
    tag: &[u8; 16],
) -> bool {
    use chacha20poly1305::aead::{AeadInOut, KeyInit};

    let cipher = chacha20poly1305::ChaCha20Poly1305::new(key.into());
    cipher
        .decrypt_inout_detached(nonce.into(), aad, buffer.into(), tag.into())
        .is_ok()
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn test_sha256_rfc_vectors() {
        assert_eq!(
            sha256(b""),
            [
                0xe3, 0xb0, 0xc4, 0x42, 0x98, 0xfc, 0x1c, 0x14, 0x9a, 0xfb, 0xf4, 0xc8, 0x99, 0x6f,
                0xb9, 0x24, 0x27, 0xae, 0x41, 0xe4, 0x64, 0x9b, 0x93, 0x4c, 0xa4, 0x95, 0x99, 0x1b,
                0x78, 0x52, 0xb8, 0x55
            ]
        );
        assert_eq!(
            sha256(b"abc"),
            [
                0xba, 0x78, 0x16, 0xbf, 0x8f, 0x01, 0xcf, 0xea, 0x41, 0x41, 0x40, 0xde, 0x5d, 0xae,
                0x22, 0x23, 0xb0, 0x03, 0x61, 0xa3, 0x96, 0x17, 0x7a, 0x9c, 0xb4, 0x10, 0xff, 0x61,
                0xf2, 0x00, 0x15, 0xad
            ]
        );
    }

    #[test]
    fn test_hmac_sha256_rfc4231_vector1() {
        let key = [0x0bu8; 20];
        let data = b"Hi There";
        let expected = [
            0xb0, 0x34, 0x4c, 0x61, 0xd8, 0xdb, 0x38, 0x53, 0x5c, 0xa8, 0xaf, 0xce, 0xaf, 0x0b,
            0xf1, 0x2b, 0x88, 0x1d, 0xc2, 0x00, 0xc9, 0x83, 0x3d, 0xa7, 0x26, 0xe9, 0x37, 0x6c,
            0x2e, 0x32, 0xcf, 0xf7,
        ];
        assert_eq!(hmac_sha256(&key, data), expected);
    }

    #[test]
    fn test_ed25519_sign_verify_roundtrip() {
        let seed = [0x42u8; 32];
        let (pk, sk) = ed25519_keypair_from_seed(&seed);
        let msg = b"AWE_OS 1.0 Production Key Certification";

        let sig = ed25519_sign(&sk, &msg[..]);
        assert!(ed25519_verify(&pk, &msg[..], &sig));

        let mut tampered_msg = msg.to_vec();
        tampered_msg[0] ^= 1;
        assert!(!ed25519_verify(&pk, &tampered_msg[..], &sig));

        let mut tampered_sig = sig;
        tampered_sig[0] ^= 1;
        assert!(!ed25519_verify(&pk, &msg[..], &tampered_sig));
    }

    #[test]
    fn test_x25519_rfc7748_vector() {
        // RFC 7748 section 5.2 test scalars.
        let alice_secret: [u8; 32] = [
            0x77, 0x07, 0x6d, 0x0a, 0x73, 0x18, 0xa5, 0x7d, 0x3c, 0x16, 0xc1, 0x72, 0x51, 0xb2,
            0x66, 0x45, 0xdf, 0x4c, 0x2f, 0x87, 0xeb, 0xc0, 0x99, 0x2a, 0xb1, 0x77, 0xfb, 0xa5,
            0x1d, 0xb9, 0x2c, 0x2a,
        ];
        let bob_secret: [u8; 32] = [
            0x5d, 0xab, 0x08, 0x7e, 0x62, 0x4a, 0x8a, 0x4b, 0x79, 0xe1, 0x7f, 0x8b, 0x83, 0x80,
            0x0e, 0xe6, 0x6f, 0x3b, 0xb1, 0x29, 0x26, 0x18, 0xb6, 0xfd, 0x1c, 0x2f, 0x8b, 0x27,
            0xff, 0x88, 0xe0, 0xeb,
        ];
        let expected_shared: [u8; 32] = [
            0x4a, 0x5d, 0x9d, 0x5b, 0xa4, 0xce, 0x2d, 0xe1, 0x72, 0x8e, 0x3b, 0xf4, 0x80, 0x35,
            0x0f, 0x25, 0xe0, 0x7e, 0x21, 0xc9, 0x47, 0xd1, 0x9e, 0x33, 0x76, 0xf0, 0x9b, 0x3c,
            0x1e, 0x16, 0x17, 0x42,
        ];

        let alice_public = x25519_public_from_secret(&alice_secret);
        let bob_public = x25519_public_from_secret(&bob_secret);

        let shared_a = x25519_shared(&alice_secret, &bob_public).expect("contributory");
        let shared_b = x25519_shared(&bob_secret, &alice_public).expect("contributory");
        assert_eq!(shared_a, expected_shared);
        assert_eq!(shared_b, expected_shared);
    }

    #[test]
    fn test_x25519_rejects_low_order_public_key() {
        let secret = [0x09u8; 32];
        let low_order = [0x00u8; 32];
        assert!(x25519_shared(&secret, &low_order).is_none());
    }

    #[test]
    fn test_hkdf_sha256_rfc5869_case1() {
        let ikm = [0x0bu8; 22];
        let salt: [u8; 13] = [
            0x00, 0x01, 0x02, 0x03, 0x04, 0x05, 0x06, 0x07, 0x08, 0x09, 0x0a, 0x0b, 0x0c,
        ];
        let info: [u8; 10] = [0xf0, 0xf1, 0xf2, 0xf3, 0xf4, 0xf5, 0xf6, 0xf7, 0xf8, 0xf9];
        let expected: [u8; 42] = [
            0x3c, 0xb2, 0x5f, 0x25, 0xfa, 0xac, 0xd5, 0x7a, 0x90, 0x43, 0x4f, 0x64, 0xd0, 0x36,
            0x2f, 0x2a, 0x2d, 0x2d, 0x0a, 0x90, 0xcf, 0x1a, 0x5a, 0x4c, 0x5d, 0xb0, 0x2d, 0x56,
            0xec, 0xc4, 0xc5, 0xbf, 0x34, 0x00, 0x72, 0x08, 0xd5, 0xb8, 0x87, 0x18, 0x58, 0x65,
        ];
        let mut out = [0u8; 42];
        assert!(hkdf_sha256(&salt, &ikm, &info, &mut out));
        assert_eq!(out, expected);
    }

    #[test]
    fn test_chacha20poly1305_rfc8439_vector() {
        // RFC 8439 section 2.8.2 AEAD test vector.
        let key: [u8; 32] = [
            0x80, 0x81, 0x82, 0x83, 0x84, 0x85, 0x86, 0x87, 0x88, 0x89, 0x8a, 0x8b, 0x8c, 0x8d,
            0x8e, 0x8f, 0x90, 0x91, 0x92, 0x93, 0x94, 0x95, 0x96, 0x97, 0x98, 0x99, 0x9a, 0x9b,
            0x9c, 0x9d, 0x9e, 0x9f,
        ];
        let nonce: [u8; 12] = [
            0x07, 0x00, 0x00, 0x00, 0x40, 0x41, 0x42, 0x43, 0x44, 0x45, 0x46, 0x47,
        ];
        let aad: [u8; 12] = [
            0x50, 0x51, 0x52, 0x53, 0xc0, 0xc1, 0xc2, 0xc3, 0xc4, 0xc5, 0xc6, 0xc7,
        ];
        let plaintext = b"Ladies and Gentlemen of the class of '99: If I could offer you only one tip for the future, sunscreen would be it.";
        let expected_first16: [u8; 16] = [
            0xd3, 0x1a, 0x8d, 0x34, 0x64, 0x8e, 0x60, 0xdb, 0x7b, 0x86, 0xaf, 0xbc, 0x53, 0xef,
            0x7e, 0xc2,
        ];
        let expected_tag: [u8; 16] = [
            0x1a, 0xe1, 0x0b, 0x59, 0x4f, 0x09, 0xe2, 0x6a, 0x7e, 0x90, 0x2e, 0xcb, 0xd0, 0x60,
            0x06, 0x91,
        ];

        let mut buffer = [0u8; 128];
        buffer[..plaintext.len()].copy_from_slice(plaintext);
        let tag = chacha20poly1305_encrypt(&key, &nonce, &aad, &mut buffer[..plaintext.len()])
            .expect("encrypt");
        assert_eq!(buffer[..16], expected_first16);
        assert_eq!(tag, expected_tag);

        assert!(chacha20poly1305_decrypt(
            &key,
            &nonce,
            &aad,
            &mut buffer[..plaintext.len()],
            &tag,
        ));
        assert_eq!(&buffer[..plaintext.len()], plaintext);

        // Authentication failure must not release plaintext.
        let mut tampered = [0u8; 128];
        tampered[..plaintext.len()].copy_from_slice(plaintext);
        let tag2 = chacha20poly1305_encrypt(&key, &nonce, &aad, &mut tampered[..plaintext.len()])
            .expect("encrypt");
        tampered[0] ^= 1;
        assert!(!chacha20poly1305_decrypt(
            &key,
            &nonce,
            &aad,
            &mut tampered[..plaintext.len()],
            &tag2,
        ));
    }
}
