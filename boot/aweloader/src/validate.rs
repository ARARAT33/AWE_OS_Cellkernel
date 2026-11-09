#![no_std]

use awe_boot_protocol::{Architecture, AWE_BOOT_VERSION};
use crate::security::{validate_manifest, Manifest};

pub fn validate_aweos(manifest: &Manifest, image_len: u64, architecture: Architecture) -> bool {
    validate_manifest(manifest, image_len, architecture)
        && manifest.kernel_entry != 0
        && match architecture {
            Architecture::X86_64 => is_canonical_x86_64(manifest.kernel_entry),
            _ => true,
        }
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
    use super::*;

    fn manifest(kernel_entry: u64) -> Manifest {
        Manifest {
            magic: crate::security::AWEOS_IMAGE_MAGIC,
            version: crate::security::AWEOS_MANIFEST_VERSION,
            header_size: core::mem::size_of::<Manifest>() as u16,
            boot_protocol: AWE_BOOT_VERSION,
            architecture: Architecture::X86_64 as u32,
            flags: crate::security::FLAG_SIGNED,
            image_offset: 0,
            image_size: 64,
            kernel_entry,
            min_boot_version: AWE_BOOT_VERSION,
            reserved: 0,
            image_digest: [0; 32],
        }
    }

    #[test]
    fn x86_64_manifest_rejects_noncanonical_kernel_entry() {
        assert!(!super::validate_aweos(
            &manifest(0x0000_8000_0000_0000),
            64,
            Architecture::X86_64,
        ));
        assert!(super::validate_aweos(
            &manifest(0xffff_8000_0000_1000),
            64,
            Architecture::X86_64,
        ));
    }

    #[test]
    fn canonical_validation_requires_sign_extension_of_bit_47() {
        assert!(is_canonical_x86_64(0x0000_7fff_ffff_ffff));
        assert!(is_canonical_x86_64(0xffff_8000_0000_0000));
        assert!(!is_canonical_x86_64(0x0000_8000_0000_0000));
        assert!(!is_canonical_x86_64(0xffff_7fff_ffff_ffff));
    }
}
