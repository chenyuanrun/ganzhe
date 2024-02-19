use crate::common::{NeU32, NeU64};

pub const MAX_STRING_LEN: usize = 256;
pub const MAX_STRING_LEN_FORCE: usize = 4096;

pub const INIT_NBDMAGIC: u64 = 0x4e42444d41474943;
pub const CLISERV_MAGIC: u64 = 0x00420281861253;

pub trait WriteRaw<Raw> {
    fn write_raw(&self, raw: &mut Raw);
}

#[repr(C)]
#[repr(packed)]
#[derive(Debug, Default, Clone)]
pub struct NegOldRaw {
    pub init_nbdmagic: NeU64,
    pub cliserv_magic: NeU64,
    pub export_size: NeU64,
    pub flags: NeU32,
    pub reserved: [NeU64; 2],
}

#[derive(Debug, Default, Clone)]
pub struct NegOld {
    pub init_nbdmagic: u64,
    pub cliserv_magic: u64,
    pub export_size: u64,
    pub flags: u32,
}

impl From<&NegOldRaw> for NegOld {
    fn from(value: &NegOldRaw) -> Self {
        Self {
            init_nbdmagic: value.init_nbdmagic.into(),
            cliserv_magic: value.cliserv_magic.into(),
            export_size: value.export_size.into(),
            flags: value.flags.into(),
        }
    }
}

impl WriteRaw<NegOldRaw> for NegOld {
    fn write_raw(&self, raw: &mut NegOldRaw) {
        raw.init_nbdmagic = self.init_nbdmagic.into();
        raw.cliserv_magic = self.cliserv_magic.into();
        raw.export_size = self.export_size.into();
        raw.flags = self.flags.into();
        raw.reserved = [0.into(), 0.into()];
    }
}
