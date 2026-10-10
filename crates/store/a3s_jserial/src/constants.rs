/// Stream magic number.
pub const STREAM_MAGIC: u16 = 0xACED;

/// Stream protocol version.
pub const STREAM_VERSION: u16 = 5;

/// First wire handle; handle `n` refers to the `n - BASE_WIRE_HANDLE`-th handle-bearing item.
pub const BASE_WIRE_HANDLE: u32 = 0x7E_0000;

/// Maximum size of a block-data chunk written by `ObjectOutputStream`.
pub const MAX_BLOCK_SIZE: usize = 1024;

/// Type codes (`TC_*`) of the stream grammar.
pub mod tc {
    pub const NULL: u8 = 0x70;
    pub const REFERENCE: u8 = 0x71;
    pub const CLASSDESC: u8 = 0x72;
    pub const OBJECT: u8 = 0x73;
    pub const STRING: u8 = 0x74;
    pub const ARRAY: u8 = 0x75;
    pub const CLASS: u8 = 0x76;
    pub const BLOCKDATA: u8 = 0x77;
    pub const ENDBLOCKDATA: u8 = 0x78;
    pub const RESET: u8 = 0x79;
    pub const BLOCKDATALONG: u8 = 0x7A;
    pub const EXCEPTION: u8 = 0x7B;
    pub const LONGSTRING: u8 = 0x7C;
    pub const PROXYCLASSDESC: u8 = 0x7D;
    pub const ENUM: u8 = 0x7E;
}

/// Class descriptor flags (`SC_*`).
pub mod flags {
    /// The class has a `writeObject` method, so its data is followed by annotations.
    pub const WRITE_METHOD: u8 = 0x01;
    pub const SERIALIZABLE: u8 = 0x02;
    pub const EXTERNALIZABLE: u8 = 0x04;
    pub const BLOCK_DATA: u8 = 0x08;
    pub const ENUM: u8 = 0x10;
}
