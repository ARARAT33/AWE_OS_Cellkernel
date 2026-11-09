#![no_std]

use awe_boot_protocol::{AWE_BOOT_VERSION, Architecture, BootInfo};

pub const AWEOS_IMAGE_MAGIC: [u8; 8] = *b"AWEOS001";
pub const AWEOS_MANIFEST_VERSION: u16 = 1;
pub const FLAG_SIGNED: u32 = 1 << 0;
pub const FLAG_RELEASE: u32 = 1 << 1;

#[repr(C)]
#[derive(Clone, Copy)]
pub struct Manifest {
    pub magic: [u8; 8],
    pub version: u16,
    pub header_size: u16,
    pub boot_protocol: u32,
    pub architecture: u32,
    pub flags: u32,
    pub image_offset: u64,
    pub image_size: u64,
    pub kernel_entry: u64,
    pub min_boot_version: u32,
    pub reserved: u32,
    pub image_digest: [u8; 32],
}

pub fn validate_manifest(m: &Manifest, image_len: u64, arch: Architecture) -> bool {
    if m.magic != AWEOS_IMAGE_MAGIC || m.version != AWEOS_MANIFEST_VERSION {
        return false;
    }
    if m.header_size as usize != core::mem::size_of::<Manifest>() {
        return false;
    }
    if m.boot_protocol != AWE_BOOT_VERSION || m.min_boot_version > AWE_BOOT_VERSION {
        return false;
    }
    if m.flags & FLAG_SIGNED == 0 || m.flags & !(FLAG_SIGNED | FLAG_RELEASE) != 0 {
        return false;
    }
    if m.reserved != 0 || m.image_size == 0 {
        return false;
    }
    let end = match m.image_offset.checked_add(m.image_size) {
        Some(v) => v,
        None => return false,
    };
    if end > image_len || m.architecture != arch as u32 {
        return false;
    }
    true
}

pub fn validate_boot_info(info: &BootInfo, arch: Architecture) -> bool {
    awe_boot_protocol::validate(info)
        && info.architecture == arch
        && info.kernel_size != 0
        && info.kernel_base.checked_add(info.kernel_size).is_some()
}

/// Cryptographic verification is intentionally not faked here. A production
/// image must be verified by a reviewed Ed25519/secure-boot implementation
/// before this policy is considered complete.
pub fn signature_required(m: &Manifest) -> bool {
    m.flags & FLAG_SIGNED != 0
}

#[cfg(test)]
mod tests {
    use super::*;

    fn valid_manifest() -> Manifest {
        Manifest {
            magic: AWEOS_IMAGE_MAGIC,
            version: AWEOS_MANIFEST_VERSION,
            header_size: core::mem::size_of::<Manifest>() as u16,
            boot_protocol: AWE_BOOT_VERSION,
            architecture: Architecture::X86_64 as u32,
            flags: FLAG_SIGNED,
            image_offset: 128,
            image_size: 64,
            kernel_entry: 0xffff_8000_0000_1000,
            min_boot_version: AWE_BOOT_VERSION,
            reserved: 0,
            image_digest: [0; 32],
        }
    }

    #[test]
    fn manifest_validation_accepts_a_well_formed_v1_header() {
        assert!(validate_manifest(
            &valid_manifest(),
            256,
            Architecture::X86_64
        ));
    }

    #[test]
    fn manifest_validation_rejects_bad_header_and_reserved_fields() {
        let mut manifest = valid_manifest();
        manifest.header_size -= 1;
        assert!(!validate_manifest(&manifest, 256, Architecture::X86_64));

        let mut manifest = valid_manifest();
        manifest.reserved = 1;
        assert!(!validate_manifest(&manifest, 256, Architecture::X86_64));

        let mut manifest = valid_manifest();
        manifest.flags |= 1 << 8;
        assert!(!validate_manifest(&manifest, 256, Architecture::X86_64));
    }

    #[test]
    fn manifest_validation_rejects_unsigned_and_out_of_range_images() {
        let mut manifest = valid_manifest();
        manifest.flags = 0;
        assert!(!validate_manifest(&manifest, 256, Architecture::X86_64));

        let mut manifest = valid_manifest();
        manifest.image_offset = u64::MAX - 8;
        assert!(!validate_manifest(&manifest, 256, Architecture::X86_64));

        let mut manifest = valid_manifest();
        manifest.image_offset = 224;
        assert!(!validate_manifest(&manifest, 256, Architecture::X86_64));
    }

    #[test]
    fn boot_info_validation_uses_the_shared_protocol_invariants() {
        let mut info = BootInfo::empty(Architecture::X86_64);
        info.kernel_base = 0x1000;
        info.kernel_size = 0x2000;
        assert!(validate_boot_info(&info, Architecture::X86_64));

        info.kernel_base = u64::MAX - 0xff;
        assert!(!validate_boot_info(&info, Architecture::X86_64));
    }
}
