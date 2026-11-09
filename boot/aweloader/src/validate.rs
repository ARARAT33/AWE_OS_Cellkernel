#![no_std]

use awe_boot_protocol::Architecture;
use crate::security::{validate_manifest, Manifest};

pub fn validate_aweos(manifest: &Manifest, image_len: u64, architecture: Architecture) -> bool {
    validate_manifest(manifest, image_len, architecture)
        && manifest.kernel_entry != 0
        && manifest.image_size != 0
        && manifest.image_offset < image_len
}

pub fn is_canonical_x86_64(address: u64) -> bool {
    let upper = address >> 48;
    if address & (1u64 << 47) == 0 {
        upper == 0
    } else {
        upper == 0xffff
    }
}

#[cfg(test)]
mod tests {
    use super::is_canonical_x86_64;

    #[test]
    fn canonical_validation_requires_sign_extension_of_bit_47() {
        assert!(is_canonical_x86_64(0x0000_7fff_ffff_ffff));
        assert!(is_canonical_x86_64(0xffff_8000_0000_0000));
        assert!(!is_canonical_x86_64(0x0000_8000_0000_0000));
        assert!(!is_canonical_x86_64(0xffff_7fff_ffff_ffff));
    }
}
